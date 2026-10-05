//! What the spectral and observation readings make of the parallel corpus.
//!
//! `_corpus/parallel` holds the Universal Declaration of Human Rights in
//! sixteen languages, the first program of ten benchmarks in each of twelve
//! programming languages, and eight SQLite scripts; `SOURCES.md` there names
//! each source. Every translation says what the others say and every program
//! does what its counterparts do, so where a reading differs from one language
//! to the next, the script and its encoding differ and the subject does not.
//!
//! Code inside prose. Every line of code is spliced into a sentence of every
//! translation, and every block of code between two of its paragraphs, each
//! after 48, 200 and 800 bytes of the translation: a reader barely warmed, half
//! warmed and settled. A line is code when something other than white space
//! is outside its comments, and a block is a run of such lines with no
//! blank or comment line inside it; a comment is prose, so a span of one would
//! not test where code begins and ends. A line goes in at a space between two
//! words, or between two characters of Chinese or Japanese, never after a
//! character that ends a clause, and a space is on either side of it. Each
//! splice opens the translation at a different paragraph and keeps 480 bytes
//! of it after the span.
//!
//! An edge is found when a change point is anywhere from 4 bytes before it to
//! 32 past it: the entry at the span's first byte, the exit at the byte after
//! its last, the exit's change point later than the entry's. `control` is the
//! share of splices whose text without the span already holds a change point
//! in that window at the splice point, the share an edge would be found with
//! no span there. The lags are read the same way out to 64 bytes, `exit64` is
//! the share of exits found that far, and `added` is how many change points a
//! span adds beyond the edges found, against the same text without it.
//! Observation's contested points are read on the same splices, an edge found
//! when one is within 16 bytes of it, beside the same reading of the text
//! without the span and the chance that a 33-byte window holds one at the
//! translation's own density, `1 - exp(-density * 33 / 1024)`.
//!
//! Each text alone. Change points and contested points a kilobyte of each
//! translation and of each language's programs; the share of frames
//! `texture_of` and `code_texture` give each texture; the share of bytes the
//! region classification of `trex spectral --classify` gives each kind; and
//! the share the lexer's blob gate collapses into opaque runs. Two controls
//! are encoded text, which the region classification reads as blobs: the
//! base64 of every translation's bytes, 76 characters to a line, and the hex
//! SHA-256 digests in the corpus's `MANIFEST.sha256`, one to a line.
//!
//! What it reads. The readings are a function of the corpus and the code, so
//! a run on any machine prints these figures.
//!
//! - 10,907 lines and 2,032 blocks of code, 621,072 splices, every span
//!   placed.
//! - A line inside a sentence: the entry is found in 52.8% of 523,536
//!   splices, the exit in 18.6%, both in 16.8%, where the text without the
//!   line holds a change point at that place in 8.4%. After 48 bytes of a
//!   translation the entry is found in 14.7%, after 800 in 77.7%. The exit is
//!   found in 19.7% to 24.7% of the Latin-script translations, 23.5% to 29.3%
//!   of the Cyrillic, 23.0% of the Greek, 17.8% of the Arabic and 18.7% of the
//!   Hebrew, 7.6% to 11.3% of the Chinese, Japanese and Korean, 8.5% of the
//!   Hindi and 15.1% of the Thai. An entry is a median 8 bytes into the
//!   line and an exit 12 past it.
//! - A block between two paragraphs: the entry is found in 90.1% of 97,536
//!   splices and the exit in 43.2%, where the text without the block holds a
//!   change point at the paragraph break in 69.2%. An exit is a median 18
//!   bytes past the block, and 51.1% are found within 64.
//! - Each text alone: 4.48 to 8.96 change points a kilobyte in the Latin,
//!   Cyrillic, Greek, Arabic and Hebrew translations, 1.62 to 4.18 in the
//!   Chinese, Japanese, Korean, Thai and Hindi, 0.94 to 3.16 in the programs.
//! - A contested point is within 16 bytes of a line's entry in 75.3% to
//!   97.0% of a translation's splices, and of its exit in 77.1% to 97.3%,
//!   where the text without the line holds one there in 15.6% to 76.1%.
//! - `texture_of` calls 99.6% to 100% of each translation's frames prose and
//!   `code_texture` 94.4% to 100%; the region classification reads at least
//!   98.4% of each translation as prose and both controls as blobs; no
//!   translation's byte is in a lexer blob run.
//!
//! Run: `cargo run --profile release-test --example what_the_parallel_corpus_reads`

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Instant;

use trex::shape::{RegionKind, classified_regions};
use trex::spectral::{CodeTexture, Texture, analyze, code_texture, texture_of};

const CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/_corpus/parallel");

/// Bytes of a translation ahead of a splice.
const DEPTHS: [usize; 3] = [48, 200, 800];

/// Bytes of a translation kept after a span.
const AFTER: usize = 480;

/// How far past an edge a change point finds it.
const FOUND: usize = 32;

/// How far past an edge a lag is read.
const LAG: usize = 64;

/// How far on either side of an edge a contested point finds it.
const CONTESTED: usize = 16;

/// The span lengths the readings are broken down by, each running from its
/// bound to the next.
const LENGTHS: [usize; 8] = [1, 16, 32, 64, 128, 256, 512, 1024];

/// The files of `dir`, in name order.
fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap_or_else(|e| panic!("{}: {e}", dir.display())).path())
        .collect();
    out.sort();
    out
}

fn name(path: &Path) -> String {
    path.file_name().and_then(|n| n.to_str()).unwrap_or_else(|| panic!("{}: no name", path.display())).to_string()
}

/// A file's text without the byte-order mark some programs open with, which
/// says how the file is encoded rather than being part of its text.
fn text(path: &Path) -> String {
    let t = fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    match t.strip_prefix('\u{feff}') {
        Some(rest) => rest.to_string(),
        None => t,
    }
}

/// One language of a set, its texts one to a file.
struct Language {
    name: String,
    texts: Vec<String>,
}

impl Language {
    /// The language's texts, a blank line between two.
    fn joined(&self) -> String {
        self.texts.iter().map(|t| t.trim_end()).collect::<Vec<_>>().join("\n\n")
    }
}

/// The translations, named by language code.
fn prose() -> Vec<Language> {
    files(&Path::new(CORPUS).join("prose"))
        .iter()
        .map(|p| Language { name: name(p).trim_end_matches(".txt").to_string(), texts: vec![text(p)] })
        .collect()
}

/// The programs by language, named by extension, and SQLite's scripts as
/// `sql`. The archive's license is the one file in `code/` that is no program.
fn code() -> Vec<Language> {
    let mut by: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let dir = Path::new(CORPUS);
    for p in files(&dir.join("code")).into_iter().chain(files(&dir.join("sql"))) {
        let Some(ext) = p.extension().and_then(|e| e.to_str()) else {
            panic!("{}: no extension to name its language", p.display());
        };
        if ext == "md" {
            println!("not a program, not read: {}", name(&p));
            continue;
        }
        by.entry(ext.to_string()).or_default().push(text(&p));
    }
    by.into_iter().map(|(name, texts)| Language { name, texts }).collect()
}

/// How a language writes comments: the markers opening one that runs to the
/// end of its line, and the pair around one that can run over several lines.
struct Comments {
    line: &'static [&'static str],
    block: Option<(&'static str, &'static str)>,
}

fn comments(lang: &str) -> Comments {
    match lang {
        "c" | "cs" | "go" | "java" | "js" | "rs" => Comments { line: &["//"], block: Some(("/*", "*/")) },
        "php" => Comments { line: &["//", "#"], block: Some(("/*", "*/")) },
        "sql" => Comments { line: &["--"], block: Some(("/*", "*/")) },
        "hs" => Comments { line: &["--"], block: Some(("{-", "-}")) },
        "lisp" => Comments { line: &[";"], block: Some(("#|", "|#")) },
        "pl" | "py" | "rb" => Comments { line: &["#"], block: None },
        other => panic!("no comment syntax for {other}"),
    }
}

/// Whether each line of `text` holds code: something other than white space
/// outside its comments. `{-#` opens a Haskell pragma, which is code.
fn code_lines(text: &str, c: &Comments) -> Vec<bool> {
    let mut inside: Option<&str> = None;
    text.lines()
        .map(|line| {
            let mut code = false;
            let mut rest = line;
            loop {
                if let Some(close) = inside {
                    let Some(i) = rest.find(close) else { break };
                    inside = None;
                    rest = &rest[i + close.len()..];
                    continue;
                }
                let t = rest.trim_start();
                if t.is_empty() {
                    break;
                }
                let line_at = c.line.iter().filter_map(|m| t.find(m)).min();
                let block_at = c.block.and_then(|(open, close)| t.find(open).map(|i| (i, open, close)));
                match (line_at, block_at) {
                    (Some(l), b) if b.is_none_or(|(b, _, _)| l < b) => {
                        code |= l > 0;
                        break;
                    }
                    (_, Some((b, open, close))) => {
                        code |= b > 0;
                        let after = &t[b + open.len()..];
                        if open == "{-" && after.starts_with('#') {
                            code = true;
                            rest = &after[1..];
                        } else {
                            inside = Some(close);
                            rest = after;
                        }
                    }
                    _ => {
                        code = true;
                        break;
                    }
                }
            }
            code
        })
        .collect()
}

/// A language's spans: every line of code, trimmed, and every block, its
/// lines' trailing white space trimmed; and its lines of code, of comment and
/// blank.
struct Spans {
    lines: Vec<String>,
    blocks: Vec<String>,
    census: [usize; 3],
}

fn spans(lang: &Language) -> Spans {
    let c = comments(&lang.name);
    let mut s = Spans { lines: Vec::new(), blocks: Vec::new(), census: [0; 3] };
    for t in &lang.texts {
        let mut run: Vec<&str> = Vec::new();
        for (l, is_code) in t.lines().zip(code_lines(t, &c)) {
            if is_code {
                s.census[0] += 1;
                s.lines.push(l.trim().to_string());
                run.push(l.trim_end());
                continue;
            }
            s.census[if l.trim().is_empty() { 2 } else { 1 }] += 1;
            if !run.is_empty() {
                s.blocks.push(run.join("\n"));
                run.clear();
            }
        }
        if !run.is_empty() {
            s.blocks.push(run.join("\n"));
        }
    }
    s
}

/// A translation, and the window opening at each of its paragraphs: the
/// paragraphs from that one on, coming round to the first after the last, a
/// blank line between two, and long enough to splice at the deepest depth past
/// the longest paragraph with `AFTER` bytes left.
struct Host {
    name: String,
    windows: Vec<String>,
}

fn host(lang: &Language) -> Host {
    let paras: Vec<&str> = lang.texts[0].split("\n\n").map(str::trim).collect();
    assert!(paras.iter().all(|p| !p.is_empty()), "{}: an empty paragraph", lang.name);
    let longest = paras.iter().map(|p| p.len()).max().expect("a translation holds a paragraph");
    let want = DEPTHS[DEPTHS.len() - 1] + longest + 2 + AFTER + 64;
    let windows = (0..paras.len())
        .map(|start| {
            let mut w = String::new();
            for p in paras.iter().cycle().skip(start) {
                if !w.is_empty() {
                    w.push_str("\n\n");
                }
                w.push_str(p);
                if w.len() >= want {
                    break;
                }
            }
            w
        })
        .collect();
    Host { name: lang.name.clone(), windows }
}

/// A character after which a span is never placed inside a sentence.
fn ends_clause(c: char) -> bool {
    matches!(c, '.' | '!' | '?' | ':' | ';' | '\u{3002}' | '\u{ff01}' | '\u{ff1f}' | '\u{ff1a}' | '\u{ff1b}' | '\u{0964}' | '\u{061f}' | '\u{0387}')
}

/// Han and kana, which are written without spaces between words.
fn unspaced(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
}

/// Where a line goes inside a sentence, at or past byte `at` of `w`: a space
/// between two words, or a boundary between two characters written without
/// spaces, neither after a character that ends a clause. The flag says
/// whether the point is a space.
fn inline_point(w: &str, at: usize) -> Option<(usize, bool)> {
    let mut prev: Option<char> = None;
    let mut chars = w.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let next = chars.peek().map(|&(_, n)| n);
        if i >= at && !prev.is_some_and(ends_clause) {
            if c == ' ' && prev.is_some_and(|p| !p.is_whitespace()) && next.is_some_and(|n| !n.is_whitespace()) {
                return Some((i, true));
            }
            if prev.is_some_and(unspaced) && unspaced(c) {
                return Some((i, false));
            }
        }
        prev = Some(c);
    }
    None
}

/// One splice: the text with the span in it, the same text without the span,
/// and where the span starts and ends.
struct Splice {
    text: String,
    control: String,
    entry: usize,
    exit: usize,
}

/// The `AFTER` bytes kept past a span, back to a character boundary.
fn after(s: &str) -> &str {
    let mut end = AFTER.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn inline(w: &str, span: &str, at: usize) -> Option<Splice> {
    let (i, space) = inline_point(w, at)?;
    let (pre, post) = if space { (w[..=i].to_string(), &w[i + 1..]) } else { (format!("{} ", &w[..i]), &w[i..]) };
    let post = after(post);
    let entry = pre.len();
    Some(Splice { text: format!("{pre}{span} {post}"), control: format!("{pre}{post}"), entry, exit: entry + span.len() })
}

/// A block goes in at the first paragraph break at or past byte `at`.
fn block(w: &str, span: &str, at: usize) -> Option<Splice> {
    let i = w.as_bytes().get(at..)?.windows(2).position(|p| p == b"\n\n")? + at;
    let (pre, post) = (&w[..i], after(&w[i + 2..]));
    let entry = pre.len() + 2;
    Some(Splice { text: format!("{pre}\n\n{span}\n\n{post}"), control: format!("{pre}\n\n{post}"), entry, exit: entry + span.len() })
}

/// The first change point from 4 bytes before `entry` to `tol` past it, and
/// the first later one from 4 bytes before `exit` to `tol` past it, each as its
/// offset from that edge.
fn edges(cuts: &[usize], entry: usize, exit: usize, tol: usize) -> (Option<i16>, Option<i16>) {
    let near = |c: usize, edge: usize| c + 4 >= edge && c <= edge + tol;
    let first = cuts.iter().copied().find(|&c| near(c, entry));
    let second = cuts.iter().copied().find(|&c| first.is_none_or(|f| c > f) && near(c, exit));
    let lag = |c: usize, edge: usize| i16::try_from(c as i64 - edge as i64).expect("a lag inside the window");
    (first.map(|c| lag(c, entry)), second.map(|c| lag(c, exit)))
}

/// What one splice read.
#[derive(Clone, Copy)]
struct Reading {
    block: bool,
    prose: u8,
    code: u8,
    depth: u8,
    length: u8,
    entry: bool,
    exit: bool,
    entry_lag: Option<i16>,
    exit_lag: Option<i16>,
    added: i32,
    contested_entry: bool,
    contested_exit: bool,
    /// The text without the span holds a change point where an edge would
    /// be found at the splice point.
    control_cut: bool,
    /// The text without the span holds a contested point within reach of the
    /// splice point.
    control_contested: bool,
}

fn small(i: usize) -> u8 {
    u8::try_from(i).expect("an index under 256")
}

/// Every splice of every span into one translation at one depth, and how many
/// spans found no place to go in.
fn read_splices(host: &Host, prose: usize, depth: usize, code: &[Spans]) -> (Vec<Reading>, usize) {
    let mut out = Vec::new();
    let mut unplaced = 0usize;
    for (ci, s) in code.iter().enumerate() {
        for (is_block, spans) in [(false, &s.lines), (true, &s.blocks)] {
            for (j, span) in spans.iter().enumerate() {
                let key = ((ci * 2 + usize::from(is_block)) * 1_000_003 + j) * DEPTHS.len() + depth;
                let w = &host.windows[key.wrapping_mul(7919) % host.windows.len()];
                let placed = if is_block { block(w, span, DEPTHS[depth]) } else { inline(w, span, DEPTHS[depth]) };
                let Some(sp) = placed else {
                    unplaced += 1;
                    continue;
                };
                let cuts = analyze(sp.text.as_bytes()).boundaries;
                let control = analyze(sp.control.as_bytes()).boundaries;
                let (entry, exit) = edges(&cuts, sp.entry, sp.exit, FOUND);
                let (entry_lag, exit_lag) = edges(&cuts, sp.entry, sp.exit, LAG);
                let contested = trex::observation::analyze(sp.text.as_bytes()).contested;
                let near = |edge: usize| contested.iter().any(|&q| q + CONTESTED >= edge && q <= edge + CONTESTED);
                // The span's first byte is at the offset where the control
                // carries on with the translation, so both edges' chance in
                // the control is read at that one offset.
                let control_contested = trex::observation::analyze(sp.control.as_bytes())
                    .contested
                    .iter()
                    .any(|&q| q + CONTESTED >= sp.entry && q <= sp.entry + CONTESTED);
                let control_cut = control.iter().any(|&c| c + 4 >= sp.entry && c <= sp.entry + FOUND);
                let matched = i32::from(entry_lag.is_some()) + i32::from(exit_lag.is_some());
                let count = |v: &[usize]| i32::try_from(v.len()).expect("a count under 2^31");
                let length = LENGTHS.iter().rposition(|&b| span.len() >= b).expect("a span of a byte or more");
                out.push(Reading {
                    block: is_block,
                    prose: small(prose),
                    code: small(ci),
                    depth: small(depth),
                    length: small(length),
                    entry: entry.is_some(),
                    exit: exit.is_some(),
                    entry_lag,
                    exit_lag,
                    added: count(&cuts) - count(&control) - matched,
                    contested_entry: near(sp.entry),
                    contested_exit: near(sp.exit),
                    control_cut,
                    control_contested,
                });
            }
        }
    }
    (out, unplaced)
}

fn pct(n: usize, of: usize) -> String {
    if of == 0 { "-".to_string() } else { format!("{:.1}", 100.0 * n as f64 / of as f64) }
}

fn per_kb(n: usize, len: usize) -> f64 {
    n as f64 * 1024.0 / len.max(1) as f64
}

#[derive(Default)]
struct Tally {
    n: usize,
    entry: usize,
    exit: usize,
    exit_late: usize,
    both: usize,
    added: i64,
    contested_entry: usize,
    contested_exit: usize,
    control_cut: usize,
    control_contested: usize,
}

impl Tally {
    fn add(&mut self, r: &Reading) {
        self.n += 1;
        self.entry += usize::from(r.entry);
        self.exit += usize::from(r.exit);
        self.exit_late += usize::from(r.exit_lag.is_some());
        self.both += usize::from(r.entry && r.exit);
        self.added += i64::from(r.added);
        self.contested_entry += usize::from(r.contested_entry);
        self.contested_exit += usize::from(r.contested_exit);
        self.control_cut += usize::from(r.control_cut);
        self.control_contested += usize::from(r.control_contested);
    }

    fn row(&self) -> String {
        let added = if self.n == 0 { 0.0 } else { self.added as f64 / self.n as f64 };
        format!(
            "{:>8} {:>6} {:>6} {:>6} {:>6} {:>7} {:>7.3}",
            self.n,
            pct(self.entry, self.n),
            pct(self.exit, self.n),
            pct(self.exit_late, self.n),
            pct(self.both, self.n),
            pct(self.control_cut, self.n),
            added
        )
    }
}

/// The readings grouped by `key`, an order and a label, inline splices beside
/// blocks.
fn table(title: &str, readings: &[Reading], key: impl Fn(&Reading) -> (usize, String)) {
    let mut by: BTreeMap<(usize, String), [Tally; 2]> = BTreeMap::new();
    for r in readings {
        by.entry(key(r)).or_default()[usize::from(r.block)].add(r);
    }
    println!("\n{title}");
    println!(
        "{:<9} {:>8} {:>6} {:>6} {:>6} {:>6} {:>7} {:>7}   {:>8} {:>6} {:>6} {:>6} {:>6} {:>7} {:>7}",
        "", "inline", "entry", "exit", "exit64", "both", "control", "added", "block", "entry", "exit", "exit64", "both", "control", "added"
    );
    for ((_, label), t) in &by {
        println!("{label:<9} {}   {}", t[0].row(), t[1].row());
    }
}

fn quantiles(mut v: Vec<i16>) -> String {
    if v.is_empty() {
        return "-".to_string();
    }
    v.sort_unstable();
    format!("median {:>3}  p90 {:>3}", v[v.len() / 2], v[(v.len() - 1) * 9 / 10])
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// `data` in base64, 76 characters a line.
fn base64(data: &[u8]) -> String {
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |a, (i, &b)| a | u32::from(b) << (16 - 8 * i));
        for k in 0..=chunk.len() {
            out.push(char::from(B64[(n >> (18 - 6 * k) & 63) as usize]));
        }
        out.extend(std::iter::repeat_n('=', 3 - chunk.len()));
    }
    out.as_bytes().chunks(76).map(|l| std::str::from_utf8(l).expect("base64 is ASCII")).collect::<Vec<_>>().join("\n")
}

/// Shares of frames by `texture_of` and by `code_texture`, of bytes by region
/// kind and of bytes in lexer blob runs.
fn texture(name: &str, input: &[u8]) {
    let field = analyze(input);
    let (mut tex, mut ctex) = ([0usize; 5], [0usize; 5]);
    for f in &field.frames {
        tex[match texture_of(f) {
            Texture::Prose => 0,
            Texture::Code => 1,
            Texture::Math => 2,
            Texture::Data => 3,
            Texture::Mixed => 4,
        }] += 1;
        ctex[match code_texture(f) {
            CodeTexture::Code => 0,
            CodeTexture::Blob => 1,
            CodeTexture::Prose => 2,
            CodeTexture::Numeric => 3,
            CodeTexture::Mixed => 4,
        }] += 1;
    }
    let mut kinds = [0usize; 6];
    for (s, e, k) in classified_regions(input) {
        kinds[RegionKind::NAMES.iter().position(|&n| n == k.label()).expect("a named kind")] += e - s;
    }
    let blob: usize = trex::lexer::blob_runs(input).iter().map(|(s, e)| e - s).sum();
    let n = field.frames.len();
    let cells = |v: &[usize], of: usize| v.iter().map(|&x| format!("{:>5}", pct(x, of))).collect::<String>();
    println!("{name:<7} {:>7}  {}  |{}  |{}  |{:>6}", input.len(), cells(&tex, n), cells(&ctex, n), cells(&kinds, input.len()), pct(blob, input.len()));
}

/// The threads to read the splices on: one a core, or one when the machine
/// cannot say how many it has.
fn threads() -> usize {
    match thread::available_parallelism() {
        Ok(n) => n.get(),
        Err(e) => {
            eprintln!("this machine's parallelism is unknown ({e}), so one thread reads");
            1
        }
    }
}

fn main() {
    let started = Instant::now();
    let prose = prose();
    let code = code();
    let spans: Vec<Spans> = code.iter().map(spans).collect();
    println!("{} translations, {} programming languages", prose.len(), code.len());
    println!("{:<5} {:>6} {:>6} {:>8} {:>8} {:>6}", "", "files", "code", "comment", "blank", "blocks");
    for (l, s) in code.iter().zip(&spans) {
        println!("{:<5} {:>6} {:>6} {:>8} {:>8} {:>6}", l.name, l.texts.len(), s.census[0], s.census[1], s.census[2], s.blocks.len());
    }

    let hosts: Vec<Host> = prose.iter().map(host).collect();
    let jobs: Vec<(usize, usize)> = (0..hosts.len()).flat_map(|p| (0..DEPTHS.len()).map(move |d| (p, d))).collect();
    let threads = threads().min(jobs.len());
    let parts: Vec<(Vec<Reading>, usize)> = thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let (jobs, hosts, spans) = (&jobs, &hosts, &spans);
                scope.spawn(move || {
                    let (mut out, mut unplaced) = (Vec::new(), 0usize);
                    for &(p, d) in jobs.iter().skip(t).step_by(threads) {
                        let begun = Instant::now();
                        let (r, u) = read_splices(&hosts[p], p, d, spans);
                        eprintln!("{} at {}: {} splices in {:.1} s", hosts[p].name, DEPTHS[d], r.len(), begun.elapsed().as_secs_f64());
                        out.extend(r);
                        unplaced += u;
                    }
                    (out, unplaced)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().expect("a splice reader panicked")).collect()
    });
    let unplaced: usize = parts.iter().map(|p| p.1).sum();
    let readings: Vec<Reading> = parts.into_iter().flat_map(|p| p.0).collect();
    println!("\n{} splices; {unplaced} spans found no place to go in", readings.len());

    println!("\n== change points at a span of code: found within {FOUND} bytes of the entry, the exit and both (%), and change points added a splice");
    table("all", &readings, |_| (0, "all".to_string()));
    table("by depth", &readings, |r| (usize::from(r.depth), DEPTHS[usize::from(r.depth)].to_string()));
    table("by span length", &readings, |r| {
        let i = usize::from(r.length);
        (i, LENGTHS.get(i + 1).map_or(format!("{}+", LENGTHS[i]), |next| format!("{}-{}", LENGTHS[i], next - 1)))
    });
    table("by translation", &readings, |r| (usize::from(r.prose), hosts[usize::from(r.prose)].name.clone()));
    table("by programming language", &readings, |r| (usize::from(r.code), code[usize::from(r.code)].name.clone()));
    println!("\nlags within {LAG} bytes, in bytes past the edge");
    for (family, is_block) in [("inline", false), ("block", true)] {
        let of = |f: fn(&Reading) -> Option<i16>| readings.iter().filter(|r| r.block == is_block).filter_map(f).collect::<Vec<_>>();
        println!("{family:<7} entry {}   exit {}", quantiles(of(|r| r.entry_lag)), quantiles(of(|r| r.exit_lag)));
    }

    println!("\n== each text alone: change points and contested points a kilobyte");
    let mut density = Vec::new();
    for (i, l) in prose.iter().chain(&code).enumerate() {
        let t = l.joined();
        let cuts = analyze(t.as_bytes()).boundaries.len();
        let contested = trex::observation::analyze(t.as_bytes()).contested.len();
        if i < prose.len() {
            density.push(per_kb(contested, t.len()));
        }
        println!("{:<5} {:>7} bytes  change points {:>6.2}  contested {:>6.2}", l.name, t.len(), per_kb(cuts, t.len()), per_kb(contested, t.len()));
    }

    println!("\n== contested points within {CONTESTED} bytes of an edge (%), against the chance a window that wide holds one");
    println!(
        "{:<5} {:>8} {:>6} {:>6} {:>7}   {:>8} {:>6} {:>6} {:>7}   {:>6}",
        "", "inline", "entry", "exit", "control", "block", "entry", "exit", "control", "chance"
    );
    let mut by: BTreeMap<u8, [Tally; 2]> = BTreeMap::new();
    for r in &readings {
        by.entry(r.prose).or_default()[usize::from(r.block)].add(r);
    }
    for (p, t) in &by {
        let p = usize::from(*p);
        let chance = 1.0 - (-density[p] * (2 * CONTESTED + 1) as f64 / 1024.0).exp();
        let side = |t: &Tally| {
            format!("{:>8} {:>6} {:>6} {:>7}", t.n, pct(t.contested_entry, t.n), pct(t.contested_exit, t.n), pct(t.control_contested, t.n))
        };
        println!("{:<5} {}   {}   {:>6.1}", hosts[p].name, side(&t[0]), side(&t[1]), 100.0 * chance);
    }

    println!("\n== texture (%): frames by texture_of | by code_texture | bytes by --classify kind | bytes in lexer blob runs");
    println!(
        "{:<7} {:>7}  {}  |{}  |{}  |{:>6}",
        "",
        "bytes",
        ["prose", "code", "math", "data", "mixed"].map(|h| format!("{h:>5}")).concat(),
        ["code", "blob", "prose", "num", "mixed"].map(|h| format!("{h:>5}")).concat(),
        RegionKind::NAMES.map(|h| format!("{:>5}", &h[..h.len().min(5)])).concat(),
        "blob"
    );
    for l in prose.iter().chain(&code) {
        texture(&l.name, l.joined().as_bytes());
    }
    let all_prose: Vec<u8> = prose.iter().flat_map(|l| l.texts[0].bytes()).collect();
    texture("base64", base64(&all_prose).as_bytes());
    let manifest = text(&Path::new(CORPUS).join("MANIFEST.sha256"));
    let digests: Vec<&str> = manifest
        .lines()
        .map(|l| {
            let d = l.split_whitespace().next().unwrap_or_else(|| panic!("MANIFEST.sha256: an empty line"));
            assert!(d.len() == 64 && d.bytes().all(|b| b.is_ascii_hexdigit()), "MANIFEST.sha256: no digest on {l:?}");
            d
        })
        .collect();
    texture("hex", digests.join("\n").as_bytes());
    eprintln!("{:.1} s", started.elapsed().as_secs_f64());
}
