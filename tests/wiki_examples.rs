//! Every documented `trex` command, run against the binary this build
//! produced.
//!
//! The wiki, the README and the command reference show commands with their
//! output. This walks every fenced `console` block in them, runs each
//! `$ trex ...` line, and compares what the binary prints with the lines the
//! document shows. A document that drifts from the engine fails here rather
//! than in a reader's terminal.
//!
//! The binary is run rather than reimplemented. Every report the commands
//! print - `--json`, `--explain`, `--count`, `--stats`, and each subcommand's
//! own - lives in `src/main.rs`, so a runner that renders through the library
//! is a second copy of all of it, and drift between the two copies is exactly
//! what this test is for.

use std::path::{Path, PathBuf};

/// One example: where it is, the command's words, the output shown, and
/// the standard input a `$ printf 'TEXT' | trex ...` line feeds it.
struct Example {
    file: PathBuf,
    line: usize,
    words: Vec<String>,
    shown: Vec<String>,
    stdin: Option<String>,
}

/// The bytes `printf FORMAT` writes, for a format holding no conversion:
/// `\n`, `\t` and `\\` are a newline, a tab and a backslash. `None` for any
/// other escape or a `%`, which this does not read.
fn printf_text(format: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = format.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                '\\' => out.push('\\'),
                _ => return None,
            },
            '%' => return None,
            c => out.push(c),
        }
    }
    Some(out)
}

/// A file a documented example reads, as a `$ cat NAME` block shows it: the
/// name it is read under and the lines beneath it. A pattern naming a set
/// (`\W{in:@words.txt}`) reads one when it is parsed, so the document that
/// shows the file is what makes its example runnable here.
struct Fixture {
    /// Where the block showing it ends, so a command is run against the
    /// files a reader has been shown by the time they reach it. A document
    /// may show one name more than once - the command reference shows
    /// notes.txt four times and rules.trex three - and each showing replaces
    /// the last from that point down, which is what reading top to bottom
    /// means.
    line: usize,
    name: String,
    contents: String,
}

/// Split a shell line into words: single-quoted strings are one word with
/// the quotes removed, everything else splits on whitespace. `None` for an
/// unterminated quote.
///
/// A `$'...'` word is the shell's other quoting, which the documents use to
/// put newlines in a `--text`, and its backslash escapes are read rather
/// than passed through. Without this the argument arrives holding a dollar,
/// two quotes and a literal backslash-n, which is a longer and different
/// input than the one the block shows.
fn shell_words(line: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '$' && {
            let mut after = chars.clone();
            after.next();
            after.next() == Some('\'')
        } {
            chars.next();
            chars.next();
            let mut word = String::new();
            loop {
                match chars.next() {
                    Some('\'') => break,
                    Some('\\') => match chars.next() {
                        Some('n') => word.push('\n'),
                        Some('t') => word.push('\t'),
                        Some('r') => word.push('\r'),
                        Some('0') => word.push('\0'),
                        Some(ch) => word.push(ch),
                        None => return None,
                    },
                    Some(ch) => word.push(ch),
                    None => return None,
                }
            }
            words.push(word);
        } else if c == '\'' {
            chars.next();
            let mut word = String::new();
            loop {
                match chars.next() {
                    Some('\'') => break,
                    Some(ch) => word.push(ch),
                    None => return None,
                }
            }
            words.push(word);
        } else {
            let mut word = String::new();
            while let Some(&ch) = chars.peek() {
                if ch.is_whitespace() {
                    break;
                }
                word.push(ch);
                chars.next();
            }
            words.push(word);
        }
    }
    Some(words)
}

/// The examples in one markdown file, the files its `$ cat NAME` blocks show,
/// and the `$ trex` lines that could not be split into words.
///
/// The third is returned rather than dropped: a command line this cannot
/// split is one nothing checks, and a count of them is the only thing that
/// says so.
fn examples_in(path: &Path) -> (Vec<Example>, Vec<Fixture>, Vec<String>) {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let mut out = Vec::new();
    let mut files = Vec::new();
    let mut unsplit = Vec::new();
    let mut in_console = false;
    let mut current: Option<Example> = None;
    let mut shown_file: Option<(usize, String, Vec<String>)> = None;
    // A command whose quote is still open, with the line it began on. A
    // `--text` argument may run over several lines, and the shell that would
    // read it goes on until the quote closes; so does this, up to the end of
    // the block, which is as far as one command can reach.
    let mut pending: Option<(usize, String)> = None;
    // Every branch below opens something new, so each first closes what is open.
    let close = |out: &mut Vec<Example>,
                 files: &mut Vec<Fixture>,
                 current: &mut Option<Example>,
                 shown_file: &mut Option<(usize, String, Vec<String>)>| {
        if let Some(ex) = current.take() {
            out.push(ex);
        }
        if let Some((line, name, mut lines)) = shown_file.take() {
            while lines.last().is_some_and(|l| l.trim().is_empty()) {
                lines.pop();
            }
            files.push(Fixture { line, name, contents: lines.join("\n") + "\n" });
        }
    };
    for (i, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            if let Some((at, cmd)) = pending.take() {
                unsplit.push(format!("{}:{at}: $ trex {cmd}", path.display()));
            }
            close(&mut out, &mut files, &mut current, &mut shown_file);
            in_console = !in_console && line.trim().starts_with("```console");
            continue;
        }
        if !in_console {
            continue;
        }
        // A line continuing an open quote is part of the command, not output,
        // so it is taken before anything else can claim it.
        if let Some((at, held)) = pending.take() {
            let cmd = format!("{held}\n{line}");
            match shell_words(&cmd) {
                Some(words) => {
                    current = Some(Example {
                        file: path.to_path_buf(),
                        line: at,
                        words,
                        shown: Vec::new(),
                        stdin: None,
                    });
                }
                None => pending = Some((at, cmd)),
            }
            continue;
        }
        if let Some(cmd) = line.strip_prefix("$ trex ") {
            close(&mut out, &mut files, &mut current, &mut shown_file);
            match shell_words(cmd) {
                Some(words) => {
                    current = Some(Example {
                        file: path.to_path_buf(),
                        line: i + 1,
                        words,
                        shown: Vec::new(),
                        stdin: None,
                    });
                }
                None => pending = Some((i + 1, cmd.to_string())),
            }
        } else if let Some(cmd) = line.strip_prefix("$ printf ") {
            close(&mut out, &mut files, &mut current, &mut shown_file);
            // `printf 'TEXT' | trex ARGS`: the binary run on ARGS with TEXT on
            // its standard input. A line of any other shape is one nothing
            // here runs, and is reported with the lines that could not be
            // split rather than passing unchecked.
            let piped = shell_words(cmd).and_then(|words| match words.as_slice() {
                [format, bar, trex, rest @ ..] if bar == "|" && trex == "trex" => {
                    printf_text(format).map(|text| (rest.to_vec(), text))
                }
                _ => None,
            });
            match piped {
                Some((words, text)) => {
                    current = Some(Example {
                        file: path.to_path_buf(),
                        line: i + 1,
                        words,
                        shown: Vec::new(),
                        stdin: Some(text),
                    });
                }
                None => unsplit.push(format!("{}:{}: $ printf {cmd}", path.display(), i + 1)),
            }
        } else if let Some(name) = line.strip_prefix("$ cat ") {
            close(&mut out, &mut files, &mut current, &mut shown_file);
            // A relative path of ordinary components, so a document can show
            // a file inside a tree it scans - `tree/logs/b.log` - and cannot
            // name anything outside the directory the examples run in. A
            // component that is a root, a prefix or `..` refuses the whole
            // name rather than being dropped from it, since a name with a
            // piece removed is a different file.
            let raw = Path::new(name.trim());
            let ordinary = raw
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)));
            if ordinary && raw.file_name().is_some() {
                shown_file = Some((i + 1, raw.to_string_lossy().into_owned(), Vec::new()));
            }
        } else if line.starts_with("$ ") {
            close(&mut out, &mut files, &mut current, &mut shown_file);
        } else if let Some(ex) = current.as_mut() {
            ex.shown.push(line.to_string());
        } else if let Some((_, _, lines)) = shown_file.as_mut() {
            lines.push(line.to_string());
        }
    }
    if let Some((at, cmd)) = pending.take() {
        unsplit.push(format!("{}:{at}: $ trex {cmd}", path.display()));
    }
    close(&mut out, &mut files, &mut current, &mut shown_file);
    (out, files, unsplit)
}

/// Every file under `dir`, as a path relative to it.
fn under(dir: &Path, prefix: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|e| panic!("entry under {}: {e}", dir.display()));
        let path = entry.path();
        let rel = prefix.join(entry.file_name());
        if path.is_dir() {
            under(&path, &rel, out);
        } else {
            out.push(rel);
        }
    }
}

/// Put the files under `from` that `examples` name into `to`, and say how
/// many were placed.
///
/// The documents read files they never show with a `$ cat` block - logs/a.log,
/// sizes.txt, hosts.txt and the rest - so those live beside this test and are
/// placed here. Only the ones a page's commands actually name are copied,
/// which keeps a page that reads none of them from carrying Moby Dick.
///
/// Placed before the page's own `$ cat` blocks are written, so a page that
/// shows a file of that name still answers for itself.
fn seed(from: &Path, to: &Path, examples: &[Example]) -> usize {
    let mut rel = Vec::new();
    under(from, Path::new(""), &mut rel);
    let mut placed = 0usize;
    for r in rel {
        let named = r.to_string_lossy().replace('\\', "/");
        let leaf = r.file_name().expect("a path from the walk names a file").to_string_lossy();
        let wanted = examples.iter().any(|ex| {
            ex.words.iter().any(|w| {
                let w = w.trim_end_matches('/');
                w == named || w == leaf || named.starts_with(&format!("{w}/"))
            })
        });
        if !wanted {
            continue;
        }
        let dst = to.join(&r);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)
                .unwrap_or_else(|e| panic!("create {}: {e}", parent.display()));
        }
        std::fs::copy(from.join(&r), &dst)
            .unwrap_or_else(|e| panic!("copy {} to {}: {e}", r.display(), dst.display()));
        placed += 1;
    }
    placed
}

/// Every markdown file under `dir`, recursively. A directory or entry that
/// cannot be read fails the test, since a page it holds would otherwise go
/// unchecked without a word.
fn markdown_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|e| panic!("entry under {}: {e}", dir.display()));
        let path = entry.path();
        if path.is_dir() {
            markdown_under(&path, out);
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
}

/// `line` with the figures a run decides replaced by a placeholder.
///
/// Three patterns cover every non-deterministic console block in README.md
/// and the wiki: a millisecond time, a seconds time and a throughput rate. Applied to the document's line and the binary's alike, so
/// an example holding one is still checked for everything except the number
/// the clock chose.
///
/// The command reference draws the same distinction in prose, under its first
/// `--stats` block: the last figure of `--stats=line`, and the two times
/// `--stats` prints, are the run's own.
///
/// A unit is only taken as one where a number runs up to it, so `28 tokens
/// lexed` and a sentence ending in a word beginning `ms` are left alone.
fn without_the_figures_a_run_decides(line: &str) -> String {
    const UNITS: [&str; 3] = [" ms", " seconds", " MB/s"];
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    'scan: while !rest.is_empty() {
        for unit in UNITS {
            let Some(after) = rest.strip_prefix(unit) else {
                continue;
            };
            let keep = out.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.').len();
            if keep < out.len() {
                out.truncate(keep);
                out.push_str("<figure>");
                out.push_str(unit);
                rest = after;
                continue 'scan;
            }
        }
        let c = rest.chars().next().expect("rest is not empty");
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// Whether `line` stands for output the document declined to repeat, as
/// `... the eight lines ...` does.
///
/// Both ends are required and a bare `...` is not one, so a line of output
/// that happens to trail off is still compared.
fn is_elision(line: &str) -> bool {
    let t = line.trim();
    t.len() > 3 && t.starts_with("...") && t.ends_with("...")
}

/// Whether `got` shows what `shown` shows.
///
/// With no elision the two have to be equal line for line, which is what
/// holds a document to the whole of its block. An elision stands for any run
/// of lines, so a block is split at each one into runs that have to appear in
/// `got` contiguously and in order; only at an elision may `got`
/// carry lines the document does not.
fn shows(shown: &[String], got: &[String]) -> bool {
    if !shown.iter().any(|l| is_elision(l)) {
        return shown == got;
    }
    let mut runs: Vec<Vec<String>> = Vec::new();
    let mut lead = false;
    let mut trail = false;
    for line in shown {
        if is_elision(line) {
            if runs.is_empty() {
                lead = true;
            }
            trail = true;
        } else {
            if trail || runs.is_empty() {
                runs.push(Vec::new());
            }
            trail = false;
            let last = runs.last_mut().expect("a run was just opened");
            last.push(line.clone());
        }
    }
    let mut at = 0usize;
    let last = runs.len().saturating_sub(1);
    for (k, run) in runs.iter().enumerate() {
        // The first run is pinned to the start unless an elision precedes it,
        // and the last to the end unless one follows it. Every run between is
        // taken at the earliest position that fits, which leaves the most
        // room for the runs after it: where any placement works, that one
        // does. Pinning the last is what an earliest-first walk cannot do on
        // its own, since the earliest match of the final run can stop before
        // the end while a later one ends exactly there.
        let pinned = if k == 0 && !lead {
            Some(0)
        } else if k == last && !trail {
            match got.len().checked_sub(run.len()) {
                Some(i) => Some(i),
                None => return false,
            }
        } else {
            None
        };
        let i = match pinned {
            Some(want) => {
                if want < at
                    || want + run.len() > got.len()
                    || &got[want..want + run.len()] != run.as_slice()
                {
                    return false;
                }
                want
            }
            None => {
                let found = (at..=got.len().saturating_sub(run.len()))
                    .find(|&i| i + run.len() <= got.len() && &got[i..i + run.len()] == run.as_slice());
                match found {
                    Some(i) => i,
                    None => return false,
                }
            }
        };
        at = i + run.len();
    }
    trail || at == got.len()
}

/// The command before a `| head -N`, and the N, where that is the whole of
/// the pipeline.
///
/// `head` takes a count of lines and no other argument, so the binary's
/// output cut to N lines is what the pipeline printed, and a command still
/// writing once N lines have arrived is stopped, as the pipe `head` closes
/// stops it: that is how a block shows the start of a follow, which never
/// ends on its own. Anything else after the bar is a program this does not
/// run, and is declined rather than guessed at.
fn piped_head(words: &[String]) -> Option<(&[String], usize)> {
    let bar = words.iter().position(|w| w == "|")?;
    let count = match &words[bar + 1..] {
        [head, count] if head == "head" => count.strip_prefix('-')?,
        [head, flag, count] if head == "head" && flag == "-n" => count.as_str(),
        _ => return None,
    };
    if count.is_empty() || !count.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((&words[..bar], count.parse().expect("a run of ascii digits is a number")))
}

/// How many whole lines a run has written to `sink` so far, a library's own
/// diagnostic left uncounted as `run` leaves it uncompared.
fn complete_lines(sink: &Path) -> Result<usize, String> {
    let raw = std::fs::read(sink).map_err(|e| format!("read {}: {e}", sink.display()))?;
    Ok(raw.split(|&b| b == b'\n').rev().skip(1).filter(|l| !l.starts_with(b"flynnel:")).count())
}

/// Why this block could not be run here, or `None` where it disagrees for a
/// reason of its own and is a real disagreement.
///
/// The missing-input test reads the binary's own words rather than guessing
/// from the command, so a block that does show a not-found message is
/// compared against it like any other output and only one that does not is
/// counted. Three spellings, because the message is the operating system's
/// and the operating system distinguishes cases this does not care about:
/// Windows says the file cannot be found for a missing leaf and the path
/// cannot be found when a directory in the way is missing, which is what
/// `logs/a.log` gives when there is no `logs`, and Unix says there is no
/// such file for both.
fn why_unrunnable(ex: &Example, shown: &[String], got: &[String]) -> Option<&'static str> {
    if ex.words.iter().any(|w| w == "|") && piped_head(&ex.words).is_none() {
        return Some("a pipeline past a head, and this runs no shell");
    }
    // A subcommand names itself in its errors - `trex compress:` where a bare
    // scan says `trex:` - so the prefix is the program and not the colon
    // after it. A document showing such a line is still compared against it,
    // because the test below requires the shown block not to hold one.
    let missing = |l: &String| {
        l.starts_with("trex")
            && (l.contains("cannot find the file")
                || l.contains("cannot find the path")
                || l.contains("No such file"))
    };
    if got.iter().any(missing) && !shown.iter().any(missing) {
        return Some("names an input no document shows");
    }
    None
}

/// Whether an example runs the coder, which only a build with the `compress`
/// feature carries: `compress`, or `seam` with one of the coder's options.
fn runs_the_coder(words: &[String]) -> bool {
    const SEAM_CODER: [&str; 11] = [
        "--compress",
        "--warm-file",
        "--warm-bytes",
        "--build-model",
        "--merge-model",
        "--model-vigilance",
        "--prune-model",
        "--model-topk",
        "--bench-coder",
        "--archive-dir",
        "--chunks",
    ];
    match words.first().map(String::as_str) {
        Some("compress") => true,
        Some("seam") => words.iter().any(|w| SEAM_CODER.contains(&w.as_str())),
        _ => false,
    }
}

/// Run one example the way a reader of the document would, from `dir`.
///
/// The binary this test's own build produced answers every subcommand and
/// every flag, so nothing here reimplements a report.
///
/// Both streams are given one handle and written to `sink`, because a block
/// shows what the terminal showed and a terminal interleaves them. Reading
/// them apart and concatenating puts every line of one after every line of
/// the other: `bpe train` prints its header to stderr and its merges to
/// stdout, so the header lands last where the block shows it first.
///
/// The exit status is not checked: a block showing an error message is
/// showing the binary's real answer to a bad input, and that answer is what
/// this holds it to.
fn run(ex: &Example, dir: &Path, sink: &Path) -> Result<Vec<String>, String> {
    // A block may pipe into `head`, which is a line count and not a shell:
    // the words before the bar are the command and the count truncates what
    // it printed. Any other pipeline is declined by `why_unrunnable`.
    let (words, keep) = match piped_head(&ex.words) {
        Some((words, n)) => (words, Some(n)),
        None => (&ex.words[..], None),
    };
    let file = std::fs::File::create(sink).map_err(|e| format!("create {}: {e}", sink.display()))?;
    let also = file.try_clone().map_err(|e| format!("a second handle on the sink: {e}"))?;
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_trex"));
    command
        .args(words)
        .current_dir(dir)
        .stdout(std::process::Stdio::from(file))
        .stderr(std::process::Stdio::from(also));
    let mut child = match &ex.stdin {
        None => command.spawn().map_err(|e| format!("launching the binary: {e}"))?,
        Some(text) => {
            let mut child = command
                .stdin(std::process::Stdio::piped())
                .spawn()
                .map_err(|e| format!("launching the binary: {e}"))?;
            let mut input = child.stdin.take().expect("a piped standard input");
            std::io::Write::write_all(&mut input, text.as_bytes())
                .map_err(|e| format!("writing the standard input: {e}"))?;
            drop(input);
            child
        }
    };
    match keep {
        Some(n) => loop {
            if child.try_wait().map_err(|e| format!("waiting for the binary: {e}"))?.is_some() {
                break;
            }
            if complete_lines(sink)? >= n {
                child.kill().map_err(|e| format!("stopping the binary: {e}"))?;
                child.wait().map_err(|e| format!("waiting for the binary: {e}"))?;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        },
        None => {
            child.wait().map_err(|e| format!("waiting for the binary: {e}"))?;
        }
    }
    let raw = std::fs::read(sink).map_err(|e| format!("read {}: {e}", sink.display()))?;
    let text = String::from_utf8_lossy(&raw).into_owned();
    // A library's own diagnostic is not the binary's answer to the block.
    // Flynnel announces on stderr when this host's stored dispatch calibration
    // disagrees with what the process measures, which is a correct use of
    // stderr and arrives on any start while that record is stale - and both
    // streams share one handle here, because a block shows what a terminal
    // showed. Named rather than matched by shape, so a line trex itself grows
    // is still compared.
    let mut lines: Vec<String> = text
        .lines()
        .filter(|l| !l.starts_with("flynnel:"))
        .map(|l| without_the_figures_a_run_decides(l.trim_end()))
        .collect();
    // Trailing blank lines go from both sides or neither. A document's come
    // from the fence that closes the block and the binary's from how its last
    // line ends, so a comparison that keeps one and drops the other fails on
    // the difference between two ways of writing nothing.
    if let Some(n) = keep {
        lines.truncate(n);
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    Ok(lines)
}

#[test]
fn the_figures_a_run_decides_are_replaced_and_nothing_else_is() {
    let cases: [(&str, &str); 6] = [
        (
            "2 matches in 1 of 1 files, 48 bytes searched, 6.967 ms",
            "2 matches in 1 of 1 files, 48 bytes searched, <figure> ms",
        ),
        ("0.000071 seconds spent lexing", "<figure> seconds spent lexing"),
        ("0.006768 seconds spent matching", "<figure> seconds spent matching"),
        (
            "  coder: logistic mix + orbit + baked prior   0.00 MB/s   (--compare for the full table)",
            "  coder: logistic mix + orbit + baked prior   <figure> MB/s   (--compare for the full table)",
        ),
        // Decimals that are the answer rather than the clock's are untouched.
        (
            "trex compress: 69 bytes -> 14 bytes  (1.541 bits/byte, 20.3% of original)",
            "trex compress: 69 bytes -> 14 bytes  (1.541 bits/byte, 20.3% of original)",
        ),
        ("28 tokens lexed", "28 tokens lexed"),
    ];
    for (given, want) in cases {
        assert_eq!(without_the_figures_a_run_decides(given), want, "over {given:?}");
    }
}

#[test]
fn two_readings_of_one_line_normalize_to_the_same_string() {
    let documented = "2 matches in 1 of 1 files, 48 bytes searched, 6.967 ms";
    let fresh = "2 matches in 1 of 1 files, 48 bytes searched, 7.412 ms";
    assert_ne!(documented, fresh);
    assert_eq!(
        without_the_figures_a_run_decides(documented),
        without_the_figures_a_run_decides(fresh)
    );
}

#[test]
fn a_quoted_argument_carries_the_newlines_it_spans() {
    // A documented `--text` may be quoted across lines, and the argument the
    // shell would hand over holds the newlines between them. The reference
    // shows this one as 32 bytes, which is what these three lines join to.
    let cmd = "echo --super --text 'alpha: one\nbravo: two\ndelta: six'";
    let words = shell_words(cmd).expect("the quote closes on the third line");
    assert_eq!(words.len(), 4, "{words:?}");
    assert_eq!(words[3], "alpha: one\nbravo: two\ndelta: six");
    assert_eq!(words[3].len(), 32);
}

#[test]
fn a_dollar_quoted_argument_has_its_escapes_read() {
    // The documents use the shell's `$'...'` to put newlines in a `--text`.
    // Passed through as written it is a dollar, two quotes and a literal
    // backslash-n: 28 bytes where the block's own output says 23.
    let cmd = "shape --text $'1,22,3\\n444,5,66\\n7,888,9'";
    let words = shell_words(cmd).expect("the quote closes");
    assert_eq!(words.len(), 3, "{words:?}");
    assert_eq!(words[2], "1,22,3\n444,5,66\n7,888,9");
    assert_eq!(words[2].len(), 23);
}

#[test]
fn an_elision_stands_for_a_run_of_lines_and_nothing_else_is_loosened() {
    let line = |s: &str| s.to_string();
    let got = vec![line("a"), line("b"), line("c"), line("d")];
    // No elision: the whole block, in order, and nothing else.
    assert!(shows(&[line("a"), line("b"), line("c"), line("d")], &got));
    assert!(!shows(&[line("a"), line("b"), line("c")], &got));
    assert!(!shows(&[line("b"), line("c"), line("d")], &got));
    // Leading, trailing and interior elisions each free their own side only.
    assert!(shows(&[line("... some ..."), line("c"), line("d")], &got));
    assert!(!shows(&[line("... some ..."), line("b"), line("c")], &got));
    assert!(shows(&[line("a"), line("b"), line("... some ...")], &got));
    assert!(shows(&[line("a"), line("... some ..."), line("d")], &got));
    assert!(!shows(&[line("a"), line("... some ..."), line("e")], &got));
    // A bare `...` is a line of output, not an elision.
    assert!(!shows(&[line("..."), line("c"), line("d")], &got));
    // A run after an elision is placed at the end rather than at its first
    // match. Taking the first leaves the block short of the output's last
    // line and reports a disagreement where there is none.
    assert!(shows(&[line("... x ..."), line("a")], &[line("a"), line("a")]));
    assert!(shows(
        &[line("a"), line("... x ..."), line("b")],
        &[line("a"), line("b"), line("q"), line("b")]
    ));
}

/// Every README and wiki page holding a `rust` block is compiled as doctests
/// through a `#[doc = include_str!(..)]` in src/lib.rs, so a Rust example is
/// held to the crate as a console example is held to the binary.
#[test]
fn every_page_with_a_rust_block_is_compiled_as_doctests() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let lib = std::fs::read_to_string(root.join("src").join("lib.rs"))
        .unwrap_or_else(|e| panic!("read src/lib.rs: {e}"));
    let mut files = vec![root.join("README.md")];
    markdown_under(&root.join("wiki").join("content"), &mut files);
    let mut pages = 0usize;
    let mut missing = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
        if !text.lines().any(|l| l.starts_with("```rust")) {
            continue;
        }
        pages += 1;
        let rel = file.strip_prefix(root).expect("every page is under the crate").to_string_lossy().replace('\\', "/");
        if !lib.contains(&format!("include_str!(\"../{rel}\")")) {
            missing.push(rel);
        }
    }
    assert!(pages > 0, "the documentation holds rust blocks");
    assert!(missing.is_empty(), "pages whose rust blocks no doctest compiles:\n{}", missing.join("\n"));
}

#[test]
fn every_documented_command_prints_what_the_document_shows() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = vec![root.join("README.md")];
    markdown_under(&root.join("wiki").join("content"), &mut files);
    let mut checked = 0usize;
    let mut verified = 0usize;
    // What could not be run here, by reason, so the hole is a figure rather
    // than a silence. A block counted here is one nothing verifies, and it
    // looks from the outside exactly like a block that passed.
    // By reason, and each one naming its blocks: a count says how large the
    // hole is and a list says where it is, and only the second can be acted
    // on.
    let mut unrun: std::collections::BTreeMap<&'static str, Vec<String>> =
        std::collections::BTreeMap::new();
    let mut failures: Vec<String> = Vec::new();
    let mut shown_files = 0usize;
    let mut seeded = 0usize;
    let mut unsplit: Vec<String> = Vec::new();
    // A pattern naming a set reads it against the working directory, so the
    // examples run from a directory holding the files their document shows.
    // One directory per document, because four names - notes.txt,
    // rules.trex, defs.trex, wrong.trex - are shown with different contents
    // by different pages, and a single directory leaves whichever was
    // written last for all of them. The binary is given the directory per
    // command rather than this process changing into it, which every other
    // test in this binary would see.
    let base = std::env::temp_dir().join(format!("trex/documented-{}", std::process::id()));
    for (k, file) in files.iter().enumerate() {
        let (examples, shown_here, bad) = examples_in(file);
        unsplit.extend(bad);
        shown_files += shown_here.len();
        let dir = base.join(format!("page-{k}"));
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("create {}: {e}", dir.display()));
        seeded += seed(&root.join("tests").join("documented"), &dir, &examples);
        let mut written = 0usize;
        for ex in &examples {
            // The files a reader has been shown by the time they reach this
            // command, written in the order the page shows them, so a name
            // shown again further down the page does not reach back up.
            while written < shown_here.len() && shown_here[written].line < ex.line {
                let f = &shown_here[written];
                let at = dir.join(&f.name);
                if let Some(parent) = at.parent() {
                    std::fs::create_dir_all(parent)
                        .unwrap_or_else(|e| panic!("create {}: {e}", parent.display()));
                }
                std::fs::write(&at, &f.contents)
                    .unwrap_or_else(|e| panic!("write {}: {e}", f.name));
                written += 1;
            }
            checked += 1;
            let mut shown: Vec<String> =
                ex.shown.iter().map(|l| without_the_figures_a_run_decides(l.trim_end())).collect();
            while shown.last().is_some_and(|l| l.trim().is_empty()) {
                shown.pop();
            }
            let at = format!(
                "{}:{}: trex {}",
                ex.file.strip_prefix(root).unwrap_or(&ex.file).display(),
                ex.line,
                ex.words.join(" ")
            );
            // A build without the coder answers a coder command by naming the
            // build that has it, and that answer is what it is held to; the
            // documented output is held to by the build with the coder.
            if !cfg!(feature = "compress") && runs_the_coder(&ex.words) {
                match run(ex, &dir, &base.join("output")) {
                    Ok(got) if got.len() == 1 && got[0].contains("cargo build --release --features compress") => {
                        verified += 1;
                    }
                    Ok(got) => failures.push(format!(
                        "{at}\n  a build without the coder does not name the build that has it\n  got:   {got:?}"
                    )),
                    Err(e) => failures.push(format!("{at}\n  {e}")),
                }
                continue;
            }
            // Kept beside the page directories rather than in one, so a
            // command reading a directory does not find it.
            match run(ex, &dir, &base.join("output")) {
                Ok(got) if shows(&shown, &got) => verified += 1,
                Ok(got) => match why_unrunnable(ex, &shown, &got) {
                    Some(why) => unrun.entry(why).or_default().push(at.clone()),
                    None => failures.push(format!("{at}\n  shown: {shown:?}\n  got:   {got:?}")),
                },
                Err(e) => failures.push(format!("{at}\n  {e}")),
            }
        }
    }
    std::fs::remove_dir_all(&base).unwrap_or_else(|e| panic!("remove {}: {e}", base.display()));
    let counted: usize = unrun.values().map(Vec::len).sum();
    println!(
        "{checked} documented commands run against the binary, {verified} verified, \
         {counted} counted as not runnable here, {shown_files} files shown, \
         {seeded} placed from tests/documented"
    );
    for (why, blocks) in &unrun {
        println!("  {:>3} {why}", blocks.len());
        for at in blocks {
            println!("        {at}");
        }
    }
    // A `$ trex` line this could not split into words is one nothing checks,
    // and it looks from the outside exactly like a line that passed.
    for line in &unsplit {
        println!("  not split into words: {line}");
    }
    assert!(checked > 40, "the documentation holds runnable examples: {checked} run");
    // The hole, ratcheted. A block counted above prints its reason only when
    // some other assertion fails, because a passing test's output is
    // captured - so the count is asserted rather than left to be read. It may
    // shrink and not grow, and a new one has to be looked at rather than
    // absorbed. Every block runs, so the hole is closed and a block that
    // stops running fails here rather than being counted quietly.
    const COUNTED: usize = 0;
    assert_eq!(
        counted, COUNTED,
        "{counted} documented commands could not be run, against {COUNTED} when this was set. \
         Something became unrunnable rather than the other way round:\n{unrun:?}"
    );
    assert!(
        failures.is_empty(),
        "{} of {checked} documented commands disagree with the binary:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
