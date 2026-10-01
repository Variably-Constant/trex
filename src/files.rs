//! The inputs a command reads and the shapes it reports them in: a walk over
//! a tree under ignore rules, the binary check, the line and column of a byte
//! offset, and a unified diff built from the edits a rewrite makes.
//!
//! A directory is walked the way ripgrep walks one: `.gitignore`, `.ignore`
//! and `.rgignore` rules apply, hidden entries are skipped, and a file holding
//! a NUL byte is binary and skipped. Each of those has a switch that widens it.
//! A file named on the command line is read whatever the rules say.

use std::io::Read;
use std::path::{Path, PathBuf};

/// What a walk skips and what it does not, and how its files are ordered.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WalkOptions {
    /// Read hidden files and directories.
    pub hidden: bool,
    /// Ignore no ignore file.
    pub no_ignore: bool,
    /// Globs a walked file is kept by (`*.rs`) or dropped by (`!*.min.js`),
    /// read against its path under the directory walked, as ripgrep's `-g`
    /// reads them; a file named on the command line is read whatever they
    /// say.
    pub globs: Vec<String>,
    /// The file types a walked file is kept by, under ripgrep's names.
    pub types: Vec<String>,
    /// The file types a walked file is dropped by.
    pub types_not: Vec<String>,
    /// How every file collected is ordered, or path order within each
    /// directory walked.
    pub sort: Option<Sort>,
}

impl WalkOptions {
    /// Whether the globs and types are ones a walk can apply.
    ///
    /// # Errors
    ///
    /// The first glob that does not parse, or the first type name the walk
    /// does not know, in ripgrep's words.
    pub fn check(&self) -> Result<(), String> {
        let mut overrides = ignore::overrides::OverrideBuilder::new(".");
        for glob in &self.globs {
            overrides.add(glob).map_err(|e| format!("glob {glob:?}: {e}"))?;
        }
        overrides.build().map_err(|e| format!("globs: {e}"))?;
        file_types(&self.types, &self.types_not)?;
        Ok(())
    }
}

/// An order for the files a walk collects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sort {
    /// What the files are ordered by.
    pub key: SortKey,
    /// Largest first.
    pub reverse: bool,
}

/// What `--sort` orders files by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    /// The path, as text.
    Path,
    /// The time the file was last written.
    Modified,
    /// The time the file was last read.
    Accessed,
    /// The time the file was created.
    Created,
}

impl SortKey {
    /// A key as `--sort` names it.
    #[must_use]
    pub fn parse(s: &str) -> Option<SortKey> {
        match s {
            "path" => Some(SortKey::Path),
            "modified" => Some(SortKey::Modified),
            "accessed" => Some(SortKey::Accessed),
            "created" => Some(SortKey::Created),
            _ => None,
        }
    }
}

/// The type filter `types` and `types_not` ask for, or none where neither
/// names one.
fn file_types(types: &[String], types_not: &[String]) -> Result<Option<ignore::types::Types>, String> {
    if types.is_empty() && types_not.is_empty() {
        return Ok(None);
    }
    let mut builder = ignore::types::TypesBuilder::new();
    builder.add_defaults();
    for t in types {
        builder.select(t);
    }
    for t in types_not {
        builder.negate(t);
    }
    builder.build().map(Some).map_err(|e| e.to_string())
}

/// ripgrep's file types, each name with its globs, as `--type-list` prints
/// them.
#[must_use]
pub fn type_list() -> Vec<(String, Vec<String>)> {
    let mut builder = ignore::types::TypesBuilder::new();
    builder.add_defaults();
    builder
        .definitions()
        .iter()
        .map(|d| (d.name().to_string(), d.globs().iter().map(ToString::to_string).collect()))
        .collect()
}

/// A file's time under `key`: none for the path key or the standard input,
/// which have no time to order by.
///
/// # Errors
///
/// The file's metadata cannot be read, or the system reports no such time
/// for it.
fn time_of(src: &Source, key: SortKey) -> Result<Option<std::time::SystemTime>, String> {
    let Source::File(path) = src else {
        return Ok(None);
    };
    let meta = std::fs::metadata(path)
        .map_err(|e| format!("{}: cannot read its metadata to sort it: {e}", path.display()))?;
    let time = match key {
        SortKey::Path => return Ok(None),
        SortKey::Modified => meta.modified(),
        SortKey::Accessed => meta.accessed(),
        SortKey::Created => meta.created(),
    };
    time.map(Some).map_err(|e| format!("{}: the system reports no such time to sort it by: {e}", path.display()))
}

/// One input a command reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// The standard input, named `-` or given when no path is.
    Stdin,
    /// A file, named or found under a directory.
    File(PathBuf),
}

impl Source {
    /// The name a report prefixes: the path, or `-` for the standard input.
    #[must_use]
    pub fn name(&self) -> String {
        match self {
            Source::Stdin => "-".to_string(),
            Source::File(p) => p.to_string_lossy().into_owned(),
        }
    }
}

/// Every file the paths name, in order: a file as itself, `-` as the
/// standard input, a directory walked under `opts` with its files in path
/// order, the whole list reordered where `opts` asks for a sort. Errors the
/// walk meets are returned beside the sources, each naming its path, rather
/// than dropped.
#[must_use]
pub fn collect(paths: &[String], opts: &WalkOptions) -> (Vec<Source>, Vec<String>) {
    let mut sources = Vec::new();
    let mut errors = Vec::new();
    let types = match file_types(&opts.types, &opts.types_not) {
        Ok(types) => types,
        Err(e) => return (sources, vec![e]),
    };
    for arg in paths {
        if arg == "-" {
            sources.push(Source::Stdin);
            continue;
        }
        let path = Path::new(arg);
        if !path.is_dir() {
            // Not a directory makes it a file only where it is there at all.
            // `is_dir` is also false for a path that is not there, so without
            // this a missing one is collected as a file: `--files` then lists
            // it as though it existed and the run exits zero, and only a
            // command that goes on to read it says otherwise. The link itself
            // is what is asked about, so a dangling symlink is collected and
            // reported by the read, which names what broke.
            match path.symlink_metadata() {
                Ok(_) => sources.push(Source::File(path.to_path_buf())),
                Err(e) => errors.push(format!("{}: {e}", path.display())),
            }
            continue;
        }
        let mut found: Vec<PathBuf> = Vec::new();
        let mut builder = ignore::WalkBuilder::new(path);
        builder
            .hidden(!opts.hidden)
            .ignore(!opts.no_ignore)
            .git_ignore(!opts.no_ignore)
            .git_global(!opts.no_ignore)
            .git_exclude(!opts.no_ignore)
            .parents(!opts.no_ignore);
        if !opts.globs.is_empty() {
            let mut overrides = ignore::overrides::OverrideBuilder::new(path);
            for glob in &opts.globs {
                if let Err(e) = overrides.add(glob) {
                    errors.push(format!("glob {glob:?}: {e}"));
                }
            }
            match overrides.build() {
                Ok(overrides) => {
                    builder.overrides(overrides);
                }
                Err(e) => errors.push(format!("{arg}: globs: {e}")),
            }
        }
        if let Some(types) = &types {
            builder.types(types.clone());
        }
        for entry in builder.build() {
            match entry {
                Ok(e) => {
                    // A tree's own index is trex's bookkeeping, not one of
                    // the tree's files: `--hidden` widens the walk to the
                    // files you hid, not to this.
                    let is_index = e.file_name() == std::ffi::OsStr::new(crate::index::INDEX_FILE);
                    if e.file_type().is_some_and(|t| t.is_file()) && !is_index {
                        found.push(e.into_path());
                    }
                }
                Err(e) => errors.push(format!("{arg}: {e}")),
            }
        }
        found.sort();
        sources.extend(found.into_iter().map(Source::File));
    }
    if let Some(sort) = opts.sort {
        // A file whose time cannot be read is reported and ordered as if it
        // had none, ahead of the rest.
        let mut keyed: Vec<(Option<std::time::SystemTime>, Source)> = Vec::with_capacity(sources.len());
        for src in sources {
            match time_of(&src, sort.key) {
                Ok(time) => keyed.push((time, src)),
                Err(e) => {
                    errors.push(e);
                    keyed.push((None, src));
                }
            }
        }
        // The standard input has no time and no path to order by, so it
        // stays ahead of every file.
        keyed.sort_by(|(ta, a), (tb, b)| {
            let (a_stdin, b_stdin) = (*a == Source::Stdin, *b == Source::Stdin);
            let order = match sort.key {
                SortKey::Path => a.name().cmp(&b.name()),
                _ => ta.cmp(tb),
            };
            b_stdin.cmp(&a_stdin).then(if sort.reverse { order.reverse() } else { order })
        });
        sources = keyed.into_iter().map(|(_, s)| s).collect();
    }
    (sources, errors)
}

/// The raw bytes of a source; `crate::encoding::decode` turns them into
/// UTF-8 text once `is_binary` has passed them.
///
/// # Errors
///
/// The file cannot be read, or the standard input cannot.
pub fn read_source(src: &Source) -> std::io::Result<Vec<u8>> {
    match src {
        Source::Stdin => {
            let mut buf = Vec::new();
            std::io::stdin().lock().read_to_end(&mut buf)?;
            Ok(buf)
        }
        Source::File(p) => std::fs::read(p),
    }
}

/// Whether the bytes open with a UTF-16 or UTF-32 byte order mark, which
/// declares them text whatever NULs they hold.
#[must_use]
pub fn has_bom(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xFE])
        || bytes.starts_with(&[0xFE, 0xFF])
        || bytes.starts_with(&[0x00, 0x00, 0xFE, 0xFF])
}

/// Whether the raw bytes hold a NUL and carry no byte order mark, which
/// marks a file binary; BOM-less UTF-16 counts as binary here.
#[must_use]
pub fn is_binary(bytes: &[u8]) -> bool {
    !has_bom(bytes) && bytes.contains(&0)
}

/// Whether the file at `path` is binary as [`is_binary`] reads its bytes,
/// read no further than its first NUL byte.
///
/// # Errors
///
/// The file cannot be opened or read.
pub fn is_binary_file(path: &Path) -> std::io::Result<bool> {
    let mut file = std::fs::File::open(path)?;
    let mut buf = vec![0u8; 64 * 1024];
    // The first four bytes are read before anything is decided, since they
    // hold the byte order mark where there is one.
    let mut head = 0;
    while head < 4 {
        match file.read(&mut buf[head..]) {
            Ok(0) => break,
            Ok(n) => head += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    if has_bom(&buf[..head]) {
        return Ok(false);
    }
    if buf[..head].contains(&0) {
        return Ok(true);
    }
    loop {
        match file.read(&mut buf) {
            Ok(0) => return Ok(false),
            Ok(n) if buf[..n].contains(&0) => return Ok(true),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
}

/// The byte offset each line of an input begins at, for turning a match
/// offset into a line and a column.
///
/// An index over a window of a longer input, a head, a tail or a range of
/// it, reports that input's own line numbers and byte offsets: [`within`]
/// says where the window stands. Its positions stay the window's own, so
/// [`line_of`] and [`line_span`] index the window; only [`line_col`],
/// [`number`] and [`offset`] give the longer input's.
///
/// [`within`]: LineIndex::within
/// [`line_of`]: LineIndex::line_of
/// [`line_span`]: LineIndex::line_span
/// [`line_col`]: LineIndex::line_col
/// [`number`]: LineIndex::number
/// [`offset`]: LineIndex::offset
#[derive(Clone, Debug)]
pub struct LineIndex {
    starts: Vec<usize>,
    len: usize,
    /// Where the input stands in a longer one: its first byte's offset;
    /// `None` for a window whose place was not counted, which only a report
    /// printing no offset is built over.
    byte_base: Option<usize>,
    /// How many lines of the longer input come before this one's first;
    /// `None` for a window whose lines ahead were not counted, which only a
    /// report printing no line number is built over.
    line_base: Option<usize>,
}

impl LineIndex {
    #[must_use]
    pub fn new(input: &[u8]) -> Self {
        let mut starts = vec![0];
        starts.extend(input.iter().enumerate().filter(|&(_, &b)| b == b'\n').map(|(i, _)| i + 1));
        LineIndex { starts, len: input.len(), byte_base: Some(0), line_base: Some(0) }
    }

    /// This index over a window of a longer input, whose first byte stands
    /// at `byte_base` of it after `line_base` of its lines, where each was
    /// counted.
    #[must_use]
    pub fn within(mut self, byte_base: Option<usize>, line_base: Option<usize>) -> Self {
        self.byte_base = byte_base;
        self.line_base = line_base;
        self
    }

    /// The one-based number a report prints for zero-based line `line`.
    ///
    /// # Panics
    ///
    /// The index is over a window whose lines ahead were not counted: a
    /// report that numbers its lines asks for them to be counted, so this is
    /// a report numbering lines it chose not to count.
    #[must_use]
    pub fn number(&self, line: usize) -> usize {
        self.line_base.expect("the lines ahead of a window are counted wherever a report numbers its lines") + line + 1
    }

    /// Offset `at` of this index's input, as an offset of the input it is a
    /// window of.
    ///
    /// # Panics
    ///
    /// The index is over a window whose place was not counted: a report that
    /// prints an offset asks for it to be counted, so this is a report
    /// printing an offset it chose not to count.
    #[must_use]
    pub fn offset(&self, at: usize) -> usize {
        self.byte_base.expect("a window is placed wherever a report prints its offsets") + at
    }

    /// Where this index's input stands in the input it is a window of, where
    /// that was counted.
    #[must_use]
    pub fn base(&self) -> Option<usize> {
        self.byte_base
    }

    /// How many lines of the input this index is a window of come before
    /// its own first line, where they were counted.
    #[must_use]
    pub fn lines_before(&self) -> Option<usize> {
        self.line_base
    }

    /// How many lines the input has: an input ending in a newline has an
    /// empty last line that is not counted, and an empty input has none.
    #[must_use]
    pub fn lines(&self) -> usize {
        match self.starts.last() {
            Some(&last) if last == self.len => self.starts.len() - 1,
            _ => self.starts.len(),
        }
    }

    /// The zero-based line an offset falls on.
    #[must_use]
    pub fn line_of(&self, offset: usize) -> usize {
        match self.starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        }
    }

    /// The byte range of a zero-based line, without its newline.
    #[must_use]
    pub fn line_span(&self, line: usize) -> (usize, usize) {
        let start = self.starts[line];
        let end = match self.starts.get(line + 1) {
            Some(&next) => next - 1,
            None => self.len,
        };
        (start, end)
    }

    /// The one-based line and one-based character column of an offset, the
    /// line numbered as the longer input numbers it where this index is over
    /// a window of one.
    #[must_use]
    pub fn line_col(&self, input: &[u8], offset: usize) -> (usize, usize) {
        let line = self.line_of(offset);
        let start = self.starts[line];
        let col = String::from_utf8_lossy(&input[start..offset]).chars().count() + 1;
        (self.number(line), col)
    }
}

/// One replacement a rewrite makes: the bytes at `start..end` become
/// `replacement`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub start: usize,
    pub end: usize,
    pub replacement: Vec<u8>,
}

/// How the text a rewrite edited was read from a file's bytes, which is what
/// the offsets of its edits count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadAs {
    /// As [`crate::encoding::decode`] decodes them.
    Decoded,
    /// As that, then read as a string reads UTF-8, each ill-formed part one
    /// U+FFFD, as `String::from_utf8_lossy` reads it.
    Lossy,
}

/// The file `raw` once `edits` are written into it: edits of its text as
/// `read` says it was read, in order and none overlapping. Every byte no
/// edit replaces is kept as it was, the byte order mark among them, and each
/// replacement is written in the encoding the file is read in.
#[must_use]
pub fn written(raw: &[u8], edits: &[Edit], read: ReadAs) -> Vec<u8> {
    let (encoding, mark) = crate::encoding::read_as(raw);
    let units = &raw[mark..];
    let mut places = crate::encoding::Places::new(units, encoding, read == ReadAs::Lossy);
    let mut out = Vec::with_capacity(raw.len());
    out.extend_from_slice(&raw[..mark]);
    let mut kept = 0;
    for e in edits {
        let start = places.latest(e.start);
        out.extend_from_slice(&units[kept..start]);
        out.extend_from_slice(&crate::encoding::encode(&e.replacement, encoding));
        kept = places.earliest(e.end);
    }
    out.extend_from_slice(&units[kept..]);
    out
}

/// The unified diff between an input and the same input with `edits`
/// applied, `context` lines around each change, hunks merged where their
/// context would overlap. Empty when there is no edit.
#[must_use]
pub fn unified_diff(path: &str, old: &[u8], edits: &[Edit], context: usize) -> String {
    if edits.is_empty() {
        return String::new();
    }
    let index = LineIndex::new(old);
    let n = index.lines();
    let mut sorted: Vec<&Edit> = edits.iter().collect();
    sorted.sort_by_key(|e| (e.start, e.end));
    let last_line = |e: &Edit| {
        if e.end > e.start { index.line_of(e.end - 1) } else { index.line_of(e.start) }
    };
    // Group edits into hunks: an edit joins the open hunk when the context
    // after the hunk would reach the context before the edit.
    let mut hunks: Vec<(usize, usize, Vec<&Edit>)> = Vec::new();
    for e in sorted {
        let first = index.line_of(e.start).min(n.saturating_sub(1));
        let last = last_line(e).min(n.saturating_sub(1));
        match hunks.last_mut() {
            Some((_, he, members)) if first <= *he + 2 * context + 1 => {
                *he = (*he).max(last);
                members.push(e);
            }
            _ => hunks.push((first, last, vec![e])),
        }
    }
    let mut out = format!("--- {path}\n+++ {path}\n");
    let mut delta: i64 = 0;
    for (hs, he, members) in hunks {
        let pre_start = hs.saturating_sub(context);
        let post_end = (he + 1 + context).min(n);
        let (old_from, _) = index.line_span(hs);
        let old_to = match index.starts.get(he + 1) {
            Some(&next) => next,
            None => old.len(),
        };
        let mut new_text: Vec<u8> = Vec::new();
        let mut cursor = old_from;
        for e in &members {
            new_text.extend_from_slice(&old[cursor..e.start]);
            new_text.extend_from_slice(&e.replacement);
            cursor = e.end;
        }
        new_text.extend_from_slice(&old[cursor..old_to]);
        let old_lines = split_lines(&old[old_from..old_to]);
        let new_lines = split_lines(&new_text);
        let pre: Vec<&[u8]> = (pre_start..hs).map(|l| line_bytes(old, &index, l)).collect();
        let post: Vec<&[u8]> = (he + 1..post_end).map(|l| line_bytes(old, &index, l)).collect();
        let old_count = pre.len() + old_lines.len() + post.len();
        let new_count = pre.len() + new_lines.len() + post.len();
        let old_start = pre_start + 1;
        let new_start = (old_start as i64 + delta).max(1) as usize;
        out.push_str(&format!("@@ -{old_start},{old_count} +{new_start},{new_count} @@\n"));
        for l in &pre {
            push_line(&mut out, ' ', l);
        }
        for l in &old_lines {
            push_line(&mut out, '-', l);
        }
        for l in &new_lines {
            push_line(&mut out, '+', l);
        }
        for l in &post {
            push_line(&mut out, ' ', l);
        }
        delta += new_lines.len() as i64 - old_lines.len() as i64;
    }
    out
}

/// The bytes of a zero-based line, without its newline.
fn line_bytes<'a>(input: &'a [u8], index: &LineIndex, line: usize) -> &'a [u8] {
    let (s, e) = index.line_span(line);
    &input[s..e]
}

/// The lines of a byte run, without their newlines; a trailing newline
/// closes the last line rather than opening an empty one.
fn split_lines(bytes: &[u8]) -> Vec<&[u8]> {
    let mut lines: Vec<&[u8]> = bytes.split(|&b| b == b'\n').collect();
    if bytes.ends_with(b"\n") {
        lines.pop();
    }
    lines
}

fn push_line(out: &mut String, mark: char, line: &[u8]) {
    out.push(mark);
    out.push_str(&String::from_utf8_lossy(line));
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_index_places_offsets() {
        let text = b"one\ntwo\n\nfour";
        let idx = LineIndex::new(text);
        assert_eq!(idx.lines(), 4);
        assert_eq!(idx.line_col(text, 0), (1, 1));
        assert_eq!(idx.line_col(text, 3), (1, 4));
        assert_eq!(idx.line_col(text, 4), (2, 1));
        assert_eq!(idx.line_col(text, 8), (3, 1));
        assert_eq!(idx.line_col(text, 10), (4, 2));
        assert_eq!(idx.line_span(1), (4, 7));
        assert_eq!(idx.line_span(3), (9, 13));
        let trailing = LineIndex::new(b"a\nb\n");
        assert_eq!(trailing.lines(), 2);
        assert_eq!(LineIndex::new(b"").lines(), 0);
        assert_eq!(LineIndex::new(b"\n").lines(), 1);
        // A column counts characters, not bytes.
        let accented = "caf\u{E9} x".as_bytes();
        assert_eq!(LineIndex::new(accented).line_col(accented, 6), (1, 6));
    }

    #[test]
    fn a_unified_diff_comes_from_the_edits() {
        let old = b"a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n";
        // `c` becomes `C` and `i` becomes two lines: two hunks under one line
        // of context, one hunk under three.
        let edits = vec![
            Edit { start: 4, end: 5, replacement: b"C".to_vec() },
            Edit { start: 16, end: 17, replacement: b"i1\ni2".to_vec() },
        ];
        let one = unified_diff("f.txt", old, &edits, 1);
        assert_eq!(
            one,
            "--- f.txt\n+++ f.txt\n@@ -2,3 +2,3 @@\n b\n-c\n+C\n d\n@@ -8,3 +8,4 @@\n h\n-i\n+i1\n+i2\n j\n"
        );
        let three = unified_diff("f.txt", old, &edits, 3);
        assert_eq!(
            three,
            "--- f.txt\n+++ f.txt\n@@ -1,10 +1,11 @@\n a\n b\n-c\n-d\n-e\n-f\n-g\n-h\n-i\n+C\n+d\n+e\n+f\n+g\n+h\n+i1\n+i2\n j\n"
        );
        assert_eq!(unified_diff("f.txt", old, &[], 3), "");
        // An edit at the very end of a file without a trailing newline.
        let tail = b"x\ny";
        let e = vec![Edit { start: 2, end: 3, replacement: b"Y".to_vec() }];
        assert_eq!(unified_diff("t", tail, &e, 0), "--- t\n+++ t\n@@ -2,1 +2,1 @@\n-y\n+Y\n");
    }

    /// Edits written into a file land in the encoding it is read in and keep
    /// every byte they do not replace: the mark, a lone surrogate, a value
    /// that is no character, the trailing part of a unit, and bytes a string
    /// could not hold.
    #[test]
    fn edits_are_written_into_the_bytes_the_file_holds() {
        use crate::encoding::{decode, read_as};

        // Each run of digits in `text` bracketed.
        fn bracketed(text: &[u8]) -> Vec<Edit> {
            let mut edits = Vec::new();
            let mut i = 0;
            while i < text.len() {
                let run = text[i..].iter().take_while(|b| b.is_ascii_digit()).count();
                if run == 0 {
                    i += 1;
                    continue;
                }
                let mut replacement = b"<".to_vec();
                replacement.extend_from_slice(&text[i..i + run]);
                replacement.push(b'>');
                edits.push(Edit { start: i, end: i + run, replacement });
                i += run;
            }
            edits
        }
        fn applied(text: &[u8], edits: &[Edit]) -> Vec<u8> {
            let mut out = Vec::new();
            let mut at = 0;
            for e in edits {
                out.extend_from_slice(&text[at..e.start]);
                out.extend_from_slice(&e.replacement);
                at = e.end;
            }
            out.extend_from_slice(&text[at..]);
            out
        }
        fn marked(mark: &[u8], body: impl IntoIterator<Item = u8>) -> Vec<u8> {
            mark.iter().copied().chain(body).collect()
        }
        let le16 = |s: &str| s.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>();
        let be16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
        let le32 = |s: &str| s.chars().flat_map(|c| u32::from(c).to_le_bytes()).collect::<Vec<u8>>();
        let be32 = |s: &str| s.chars().flat_map(|c| u32::from(c).to_be_bytes()).collect::<Vec<u8>>();

        let text = "one 1\nd\u{E9}j\u{E0} 22\n\u{1F600} 333 \u{4E2D}\n";
        let plain = "one 1\ntwo 22\nthree 333\n";
        let files: Vec<Vec<u8>> = vec![
            text.as_bytes().to_vec(),
            marked(&[0xEF, 0xBB, 0xBF], text.bytes()),
            marked(&[0xFF, 0xFE], le16(text)),
            marked(&[0xFE, 0xFF], be16(text)),
            marked(&[0xFF, 0xFE, 0, 0], le32(text)),
            marked(&[0, 0, 0xFE, 0xFF], be32(text)),
            le16(plain),
            be16(plain),
        ];
        for raw in &files {
            let read = decode(raw.clone());
            let edits = bracketed(&read);
            assert_eq!(edits.len(), 3, "{:02X?}", &raw[..4]);
            assert_eq!(written(raw, &[], ReadAs::Decoded), *raw);
            let out = written(raw, &edits, ReadAs::Decoded);
            assert_eq!(read_as(&out), read_as(raw), "{:02X?}", &raw[..4]);
            assert_eq!(decode(out), applied(&read, &edits), "{:02X?}", &raw[..4]);
        }

        // A lone surrogate stays as the unit it is, though it reads as U+FFFD.
        let lone = [&[0xFF, 0xFE][..], &le16("a "), &[0x00, 0xD8], &le16(" 1\n")].concat();
        let out = written(&lone, &bracketed(&decode(lone.clone())), ReadAs::Decoded);
        assert_eq!(out, [&[0xFF, 0xFE][..], &le16("a "), &[0x00, 0xD8], &le16(" <1>\n")].concat());

        // A value that is no character reads as nothing and stays on its side
        // of the edits either side of it.
        let invalid = [0x00, 0x00, 0x11, 0x00];
        let odd = [&[0xFF, 0xFE, 0, 0][..], &le32("a"), &invalid, &le32(" 1\n")].concat();
        let a = vec![Edit { start: 0, end: 1, replacement: b"A".to_vec() }];
        assert_eq!(
            written(&odd, &a, ReadAs::Decoded),
            [&[0xFF, 0xFE, 0, 0][..], &le32("A"), &invalid, &le32(" 1\n")].concat()
        );
        let space = vec![Edit { start: 1, end: 2, replacement: b"_".to_vec() }];
        assert_eq!(
            written(&odd, &space, ReadAs::Decoded),
            [&[0xFF, 0xFE, 0, 0][..], &le32("a"), &invalid, &le32("_1\n")].concat()
        );

        // The trailing part of a unit stays after the last edit.
        let cut = [&[0xFE, 0xFF][..], &be16("x 7"), &[0x41]].concat();
        let out = written(&cut, &bracketed(&decode(cut.clone())), ReadAs::Decoded);
        assert_eq!(out, [&[0xFE, 0xFF][..], &be16("x <7>"), &[0x41]].concat());

        // Bytes that are not UTF-8 stay, whether the edits count them as they
        // are or as the U+FFFD a string holds for each.
        let latin = b"caf\xE9 1\nna\xEFve 22\n".to_vec();
        let want = b"caf\xE9 <1>\nna\xEFve <22>\n".to_vec();
        assert_eq!(written(&latin, &bracketed(&latin), ReadAs::Decoded), want);
        let lossy = String::from_utf8_lossy(&latin).into_owned();
        assert_eq!(written(&latin, &bracketed(lossy.as_bytes()), ReadAs::Lossy), want);
        assert_eq!(written(&latin, &[], ReadAs::Lossy), latin);
    }

    #[test]
    fn binary_is_a_nul_anywhere_outside_a_bom() {
        assert!(!is_binary(b"plain text\n"));
        assert!(is_binary(b"text then \0 later"));
        assert!(is_binary(&[0]));
        assert!(!is_binary(b""));
        assert!(is_binary(b"h\0i\0\n\0"), "BOM-less UTF-16 is binary");
        assert!(!is_binary(&[0xFF, 0xFE, b'h', 0, b'i', 0]), "UTF-16 LE BOM");
        assert!(!is_binary(&[0xFE, 0xFF, 0, b'h', 0, b'i']), "UTF-16 BE BOM");
        assert!(!is_binary(&[0xFF, 0xFE, 0, 0, b'h', 0, 0, 0]), "UTF-32 LE BOM");
        assert!(!is_binary(&[0, 0, 0xFE, 0xFF, 0, 0, 0, b'h']), "UTF-32 BE BOM");
    }

    /// A path that is not there is an error the walk met, not a file.
    ///
    /// Collected as a file it reaches `--files`, which prints the names it
    /// would search without reading them, so the run lists a path that does
    /// not exist and exits zero.
    #[test]
    fn a_path_that_is_not_there_is_an_error_rather_than_a_source() {
        let root = std::env::temp_dir().join(format!("trex/absent-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("the test's directory is made");
        let here = root.join("here.log");
        std::fs::write(&here, "x").expect("the one real file is written");
        let ask = |p: &std::path::Path| {
            collect(&[p.to_string_lossy().into_owned()], &WalkOptions::default())
        };

        let (sources, errors) = ask(&here);
        assert_eq!(sources.len(), 1, "a file that is there is a source");
        assert!(errors.is_empty(), "{errors:?}");

        for absent in [root.join("gone.log"), root.join("gone")] {
            let (sources, errors) = ask(&absent);
            assert!(sources.is_empty(), "{} became a source: {sources:?}", absent.display());
            assert_eq!(errors.len(), 1, "{} reported no error", absent.display());
            assert!(
                errors[0].contains("gone"),
                "the error names the path it is about: {errors:?}"
            );
        }
        std::fs::remove_dir_all(&root).expect("the test's directory is removed");
    }

    #[test]
    fn a_walk_reads_what_ripgrep_reads() {
        let root = std::env::temp_dir().join(format!("trex/walk-{}", std::process::id()));
        let make = |rel: &str, body: &str| {
            let p = root.join(rel);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).expect("the test tree's directories are made");
            }
            std::fs::write(&p, body).expect("the test tree's files are written");
        };
        make("keep.log", "x");
        make("sub/deep.txt", "y");
        make("sub/.hidden", "z");
        make("build/out.o", "\0\0");
        make(".ignore", "build/\n");
        let (plain, errors) = collect(&[root.to_string_lossy().into_owned()], &WalkOptions::default());
        assert!(errors.is_empty(), "{errors:?}");
        let names: Vec<String> = plain
            .iter()
            .map(|s| s.name().replace('\\', "/"))
            .map(|n| n[n.rfind(root.file_name().and_then(|f| f.to_str()).expect("a name")).expect("under root")..].to_string())
            .collect();
        assert!(names.iter().any(|n| n.ends_with("keep.log")), "{names:?}");
        assert!(names.iter().any(|n| n.ends_with("sub/deep.txt")), "{names:?}");
        assert!(!names.iter().any(|n| n.ends_with(".hidden")), "hidden skipped: {names:?}");
        assert!(!names.iter().any(|n| n.ends_with("out.o")), "ignored skipped: {names:?}");
        let (wide, _) = collect(
            &[root.to_string_lossy().into_owned()],
            &WalkOptions { hidden: true, no_ignore: true, ..WalkOptions::default() },
        );
        assert!(wide.iter().any(|s| s.name().ends_with(".hidden")));
        assert!(wide.iter().any(|s| s.name().ends_with("out.o")));
        // A file named directly is read whatever the rules say, and `-` is
        // the standard input.
        let (named, _) = collect(
            &[root.join("build/out.o").to_string_lossy().into_owned(), "-".to_string()],
            &WalkOptions::default(),
        );
        assert_eq!(named.len(), 2);
        assert_eq!(named[1], Source::Stdin);
        std::fs::remove_dir_all(&root).expect("the test tree is removed");
    }

    #[test]
    fn a_file_is_binary_exactly_when_its_bytes_are() {
        let root = std::env::temp_dir().join(format!("trex/binary-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("the test's directory is made");
        // A NUL past the first read, so the check reads on past it.
        let mut late = vec![b'a'; 200_000];
        late.push(0);
        let cases: [(&str, Vec<u8>); 7] = [
            ("empty", Vec::new()),
            ("short", b"ab".to_vec()),
            ("zero", b"\0".to_vec()),
            ("text", b"hello world\n".to_vec()),
            ("late", late),
            ("utf16", vec![0xFF, 0xFE, b'a', 0, b'b', 0]),
            ("utf32", vec![0, 0, 0xFE, 0xFF, 0, 0, 0, b'a']),
        ];
        for (name, bytes) in &cases {
            let p = root.join(name);
            std::fs::write(&p, bytes).expect("the test's file is written");
            assert_eq!(is_binary_file(&p).expect("a readable file"), is_binary(bytes), "{name}");
        }
        assert!(is_binary_file(&root.join("absent")).is_err());
        std::fs::remove_dir_all(&root).expect("the test's directory is removed");
    }
}
