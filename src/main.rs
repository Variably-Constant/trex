//! trex command-line entry point.
//!
//! `trex scan PATTERN (FILE | --text STRING)` tokenizes the input,
//! compiles the pattern, runs the engine, and prints each match
//! span with its captured registers. `--json` emits machine
//! output; `--require-match` makes a no-match run exit non-zero.

use std::process::ExitCode;

pub(crate) use trex::report::{ValueView, json_escape, match_json as json_match_full};

/// Read an input file in any UTF encoding (see [`trex::encoding::decode`]):
/// a BOM selects UTF-32 LE/BE, UTF-16 LE/BE, or UTF-8; BOM-less UTF-16 is
/// detected only on strict, unambiguous evidence; everything else - valid
/// UTF-8, ASCII, binary - passes through unchanged.
fn read_input<P: AsRef<std::path::Path>>(path: P) -> std::io::Result<Vec<u8>> {
    Ok(trex::encoding::decode(std::fs::read(path)?))
}

mod cli_files;
mod cli_follow;
mod cli_lines;
mod cli_rules;
mod cli_window;
mod out;

/// Where Flynnel's notes go: nowhere, since the console carries only what a
/// command prints and trex keeps no log to write them to.
fn flynnel_note(_note: &str) {}

fn main() -> ExitCode {
    // Installed before any command runs, so the notes of the first parallel
    // run, a calibration Flynnel measured among them, reach it.
    trex::notice::set_sink(Some(flynnel_note));
    // A reader closing the pipe early (`trex ... | head`) is normal use, not an
    // error: println! panics on the closed pipe, so exit 0 quietly instead of
    // dumping a panic. One hook here keeps every command pipe-safe.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let broken_pipe = info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| info.payload().downcast_ref::<&str>().copied())
            .is_some_and(|s| s.contains("failed printing to stdout"));
        if broken_pipe {
            std::process::exit(0);
        }
        default_hook(info);
    }));

    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(msg) = take_clock_args(&mut args) {
        eprintln!("trex: {msg}");
        return ExitCode::FAILURE;
    }

    match args.first().map(String::as_str) {
        Some("--version" | "-V") => {
            println!("trex {}", trex::version());
            ExitCode::SUCCESS
        }
        Some("scan") => cli_files::run_scan(&args[1..]),
        Some("head") => cli_lines::run_listing(cli_lines::Listing::Head, &args[1..]),
        Some("tail") => cli_lines::run_listing(cli_lines::Listing::Tail, &args[1..]),
        Some("lines") => cli_lines::run_listing(cli_lines::Listing::Lines, &args[1..]),
        Some("rewrite") => cli_files::run_rewrite(&args[1..]),
        Some("redact") => cli_files::run_redact(&args[1..]),
        Some("templates") => cli_files::run_templates(&args[1..]),
        Some("infer") => cli_files::run_infer(&args[1..]),
        Some("count-by") => cli_files::run_aggregate(cli_files::Aggregate::CountBy, &args[1..]),
        Some("top") => cli_files::run_aggregate(cli_files::Aggregate::Top, &args[1..]),
        Some("uniq") => cli_files::run_aggregate(cli_files::Aggregate::Uniq, &args[1..]),
        Some("index") => cli_files::run_index(&args[1..]),
        Some("lib") => run_lib(&args[1..]),
        Some("grammar") => run_grammar(&args[1..]),
        Some("bpe") => run_bpe(&args[1..]),
        Some("prefilter") => run_prefilter(&args[1..]),
        Some("spectral") => run_spectral(&args[1..]),
        Some("shape") => run_shape(&args[1..]),
        Some("magnitude") => run_magnitude(&args[1..]),
        Some("stress") => run_stress(&args[1..]),
        Some("echo") => run_echo(&args[1..]),
        Some("relation") => run_relation(&args[1..]),
        Some("flow") => run_flow(&args[1..]),
        Some("observe") => run_observe(&args[1..]),
        Some("seam") => run_seam(&args[1..]),
        Some("orbit") => run_orbit(&args[1..]),
        Some("compress") => run_compress(&args[1..]),
        Some("--help" | "-h") | None => {
            print_usage();
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("trex: unrecognized argument: {other}");
            eprintln!("run `trex --help` for usage");
            ExitCode::FAILURE
        }
    }
}

/// Take `--now VALUE`, `--tz VALUE` and `--date-order VALUE` out of the
/// arguments, wherever they stand, and set what every scan in this process
/// reads a timestamp by. `--now` is a timestamp with a date in any form the
/// lexer recognizes or seconds since the epoch; `--tz` is the offset a
/// timestamp written with no zone is read in; `--date-order` says which of
/// the two leading fields of an all-numeric slash date is the day where both
/// readings are dates the calendar holds.
fn take_clock_args(args: &mut Vec<String>) -> Result<(), String> {
    let mut i = 0;
    while i < args.len() {
        if !matches!(args[i].as_str(), "--now" | "--tz" | "--date-order") {
            i += 1;
            continue;
        }
        let flag = args.remove(i);
        if i >= args.len() {
            return Err(format!("{flag} needs a value"));
        }
        let value = args.remove(i);
        match flag.as_str() {
            "--now" => {
                let secs = trex::typed::parse_now_arg(&value).ok_or_else(|| {
                    format!("--now {value:?} is neither a timestamp with a date nor seconds since the epoch")
                })?;
                trex::set_now(Some(secs));
            }
            "--tz" => {
                let offset = trex::typed::parse_tz_arg(&value).ok_or_else(|| {
                    format!("--tz {value:?} is not a zone offset (+02:00, -0530, Z)")
                })?;
                trex::set_tz_offset(offset);
            }
            _ => {
                let day_first = trex::typed::parse_date_order_arg(&value).ok_or_else(|| {
                    format!("--date-order {value:?} is neither dmy nor mdy")
                })?;
                trex::set_date_order_day_first(day_first);
            }
        }
    }
    Ok(())
}

/// The value of the flag at `args[i - 1]`, which is `args[i]`, read as a `T`.
///
/// A value missing or unreadable is an error naming the flag and what it was
/// given. One that fell back to a default would run a different command from
/// the one typed and say nothing: `--limit 1O` would scan with no limit.
fn flag_value<T>(args: &[String], i: usize, flag: &str) -> Result<T, String>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let Some(v) = args.get(i) else {
        return Err(format!("{flag} needs a value"));
    };
    v.parse().map_err(|e| format!("{flag} {v:?} is not a value it takes: {e}"))
}

/// [`flag_value`], or the command named `$command` failing with its message.
macro_rules! flag_value_or_fail {
    ($command:expr, $args:expr, $i:expr, $flag:expr) => {
        match flag_value($args, $i, $flag) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("trex {}: {e}", $command);
                return ExitCode::FAILURE;
            }
        }
    };
}

fn print_usage() {
    println!("trex {}", trex::version());
    println!("Token-Regular EXpression: regex-shaped patterns over typed tokens.");
    println!();
    println!("USAGE:");
    println!("    trex scan PATTERN FILE            scan a file for a pattern");
    println!("    trex scan PATTERN DIR             scan a tree: .gitignore, .ignore and hidden");
    println!("                                      entries skipped, binary files skipped");
    println!("      [FILE|DIR|-]...                 any number of inputs; `-` or none is stdin");
    println!("      -A N -B N -C N                  lines after, before, around each match");
    println!("      --count --count-matches -l      lines holding a match, or matches, per file;");
    println!("                                      files with a match");
    println!("      -H  --no-filename               prefix path:line:col:, or never");
    println!("      --hidden --no-ignore --binary   read what the walk skips");
    println!("    trex scan PATTERN --text STRING   scan an inline string");
    println!("      --shape 'name = `bytepat`'      declare a token shape, tried before the");
    println!("                                      built-in recognizers; match it as \\{{name}}");
    println!("      --shape-after 'name = `pat`'    the same, tried only where no built-in matched");
    println!("      --now TIMESTAMP                 the instant a \\T{{age<24h}} or \\T{{<now}} clause reads");
    println!("      --tz OFFSET                     the zone a timestamp written with none is read in");
    println!("      --explain                       under each match: the kinds it spans, the guard each");
    println!("                                      passed, every axis value read there, the route that answered");
    println!("      --format TEMPLATE               one line per match from a template: ${{0}}, ${{name:acc}},");
    println!("                                      ${{path}}, ${{line}}, ${{col}}, ${{start}}, ${{end}}; \\t and \\n");
    println!("      --head N --tail N --lines A..B  scan only those records of each input, lines unless");
    println!("                                      --record names another unit; the input's own line numbers");
    println!("      --follow                        then scan what each file gains as it grows");
    println!("    trex head N [INPUT]...            the first N lines of each input; -n numbers them");
    println!("    trex tail N [INPUT]... [-f]       the last N lines, read backward from the end; -f follows");
    println!("                                      each file as it grows, through truncation and rotation");
    println!("    trex lines A..B [INPUT]...        lines A through B; A.. to the end, ..B from the start");
    println!("      --record UNIT                   count paragraphs, blocks or another record unit instead");
    println!("    trex rewrite PATTERN TEMPLATE (FILE | --text STRING)");
    println!("                                      replace each match with a rendered template");
    println!("    trex rewrite PATTERN TEMPLATE [FILE|DIR]... --in-place [--dry-run] [-C N]");
    println!("                                      rewrite every file with a match in place, or");
    println!("                                      print the unified diff that would");
    println!("      --head N --tail N --lines A..B  rewrite only there; printed, the window alone,");
    println!("                                      in place, the file keeping every other line");
    println!("      --lib FILE                      declarations for \\{{name}}: let, kind, shape, test lines");
    println!("    trex redact PATTERN [INPUT]...    mask every match, one * per character");
    println!("      --keep 'card:last4, ip:octet1-2' leave the named fields where they stand");
    println!("      --mask C                        another character, or a token per masked run");
    println!("    trex templates [INPUT]...         each distinct record shape once with its count,");
    println!("                                      varying positions as <kind>; --pattern spells");
    println!("                                      it as a pattern; --rare [--cut N|P%] the rare ones");
    println!("      --record UNIT                   group records rather than lines, as a query does");
    println!("      --against FILE [--novel]        mark each shape shared or novel against another");
    println!("                                      input, or print only the novel ones");
    println!("    trex infer EXAMPLE EXAMPLE...     the most specific pattern every example matches,");
    println!("                                      verified against each; -f FILE reads examples");
    println!("                                      one per line; --anchored adds ^ and $");
    println!("    trex top PATTERN KEY [INPUT]...   group the matches by a rendered key, most");
    println!("                                      frequent first; KEY is a rewrite template");
    println!("    trex count-by PATTERN KEY ...     the same table, ordered by key");
    println!("    trex uniq PATTERN KEY ...         the distinct keys alone");
    println!("      -n N                            show only the first N rows");
    println!("    trex index DIR...                 index a tree so a later scan opens only the");
    println!("                                      files that can match; --list reports one");
    println!("    trex lib [--json]                 the shipped library of named patterns");
    println!("    trex lib --test [FILE...]         run the test lines of pattern files, or the");
    println!("                                      shipped library's own where none is named");
    println!("    trex prefilter (FILE | --text STRING) [--literal L]...");
    println!("                                      report which literals might occur");
    println!("    trex grammar GRAMMAR (FILE | --text STRING) [--start RULE]");
    println!("                                      parse input against a token grammar");
    println!("                                      [--count|--best|--prob] semiring values");
    println!("                                      [--segment TEXT --dict FILE] parse over a");
    println!("                                      segmentation lattice of run-together text");
    println!("    trex bpe train CORPUS [--merges K] [--max-bytes N]");
    println!("                                      learn a subword (BPE) tokenizer from a corpus");
    println!("    trex bpe encode (FILE | --text S) --model M");
    println!("                                      segment text into learned subwords");
    println!();
    println!("SPECTRAL - the temporal substrate: a third scale riding bytes -> tokens:");
    println!("    trex spectral (FILE | --text STRING)  filterbank/entropy/period/novelty + change-points");
    println!("                                      [--segment] change-point boundaries (the spectral subtokens)");
    println!("                                      [--bands] per-hop band table   [--classify] texture timeline");
    println!("                                      [--json] machine-readable frames + boundaries");
    println!("    trex seam (FILE | --text STRING)  bidirectional predictive segmentation: cut where the past");
    println!("                                      stops predicting the future; [--order K] [--field] [--segment]");
    println!("                                      [--recover [--compare-bpe]] word-boundary recovery vs count-BPE");
    println!("    trex compress (FILE | --text STRING)  report achieved compression of the default coder");
    println!("                                      (logistic mix + orbit, auto-using the baked prior);");
    println!("                                      [--compare] full coder table  [--no-baked] corpus-free");
    println!("                                      [--chunks N] N-core chunked (0 = all cores)");
    println!("                                      [--gpu] the device coder  [--hybrid] CPU + GPU concurrent");
    println!("                                      in a build with the compress feature only:");
    println!("                                      cargo build --release --features compress");
    println!("    trex echo (FILE | --text STRING)  the recurrence axis: per-token echo count/lag/period,");
    println!("                                      novelty + echo rate; [--field] per-token detail");
    println!("                                      [--orbit case|shape|notation|ip|url|time|path|fold|numeric]");
    println!("                                      recurrence up to a symmetry or a representation");
    println!("                                      [--super] recurring supertoken structures (structural rhyme)");
    println!("    trex relation (FILE | --text STRING)  the geometric tier: directed relations (encloses /");
    println!("                                      operator / adjacent), nesting load, holonomy, holography");
    println!("                                      (boundary reconstructs bulk), Ricci curvature, topology");
    println!("                                      (Betti numbers), geodesic shortcuts, entanglement min-cut;");
    println!("                                      [--edges] edges + chords  [--field] enclosure  [--gauge] canonical form");
    println!("    (every listing prints in full; --limit N bounds one, and anything cut is marked '+N more')");
    println!();
    println!();
    println!("    trex --version                    print the version");
    println!("    trex --help                       print this message");
    println!();
    println!("SCAN FLAGS:");
    println!("    --json            emit matches as JSON");
    println!("    --require-match   exit non-zero when nothing matches");
    println!("    -v -x -o -m N     the lines no match touches; whole-line matches only;");
    println!("                      the matched text (the default); at most N matches an input");
    println!("    -e P -f FILE      patterns to join as an alternation; a file of them, one a line");
    println!("    -g GLOB -t TYPE   keep walked files by glob (`!` drops) or type; -T drops a type;");
    println!("                      --type-list names the types");
    println!("    -L --files        the inputs with no match; the files a scan reads, unscanned");
    println!("    --sort KEY        path, modified, accessed or created; --sortr reverses");
    println!("    --color WHEN      always, never, auto, or 16, 256, truecolor; --colors ROLE:fg:COLOR");
    println!("    --stats[=line]    counts and times after the report, in eight lines or one");
    println!("    --passthru        every line, the matches painted");
    println!("    --all --any --none --at-least N   a record query over the -e patterns; --not P must be absent");
    println!("    --record UNIT     line, paragraph, file, period, seam, bind[:Q], auto, texture, shape,");
    println!("                      block, unit[:ROLE];");
    println!("                      --record-start P runs a record match to match, --record-span P is each match");
    println!("    --chunk-size N    feed the input in N-byte chunks (streaming scan)");
    println!("    --dual-grain      run the byte grain and token grain as a pipeline");
    println!("    (backend)         default: auto-route a large eligible scan to a present device");
    println!("    --gpu             force the SIMT device backend (warns and falls back to CPU)");
    println!("    --cpu, --nogpu    force the CPU engine (never probe the device)");
    println!();
    println!("PREFILTER FLAGS:");
    println!("    --literal L       a literal to test (repeatable)");
    println!("    --filter NAME     bloom (default), cuckoo, or xor");
    println!("    --verify          run a zero-false-negative check over the corpus");
    println!();
    println!("BINDING & BACKREFERENCE:");
    println!("    P:name  bind      =name  backref      ::name  scoped bind (to \\B group)");
    println!("    =shape name / =case name / =notation name  fuzzy backref (up to a symmetry)");
    println!();
    println!("REWRITE TEMPLATE:");
    println!("    ${{name}}          render a named capture; ${{0}} the whole match");
    println!("    ${{name:upper}}    transform a capture (upper, lower, trim)");
    println!("    ${{pair.k}}        a register bound inside a bound pattern, named through it");
    println!("    ${{k[0]}}          one binding of a register bound under a repetition");
    println!("    $$               a literal dollar sign");
    println!();
    println!("STRUCTURAL LENSES (language-agnostic pattern macros):");
    println!("    @call @block @nesting @string @number @ident @assignment");
    println!("    @kv @flag @list @range");
    println!();
    println!("TYPED ATOMS (lexer-recognized alphabet):");
    println!("    \\N num \\W word \\Q quoted \\I ip \\U url \\E email \\T time \\P punct");
    println!("    \\V semver \\G uuid \\{{mac}} mac \\H hexcolor \\C cidr \\% percent \\Z bytesize \\$ money");
    println!("    anchors: ^ $ line start/end, \\A \\z input start/end");
    println!("    \\D hash \\R duration \\L path");
    println!();
    println!("AXIS PREDICATES & ANCHORS (match by computed property, not content):");
    println!("    \\M{{>6}}  magnitude (scale)        \\N{{>6}}  a number of that scale");
    println!("    \\N{{>+1}}  an order above the window before it   \\N{{>+2s}}  two sigmas above");
    println!("    \\N{{>+1:phase}}  above its column   \"key\":k \"=\" \\N{{>+1:k}}  above the key's history");
    println!("    \\F{{entropy>0.8}} \\F{{period}} \\F{{texture:code}}  spectral");
    println!("    =shape x  =case x  =notation x   fuzzy backreference (orbit)");
    println!("    #\"W(W,W)\"  silhouette (structural form)   !~\"lit\"  negative lookahead");
    println!("    @seam (predictive break)  @nested>k (deep nesting)  @ambiguous (vantage)");
    println!("    @phase:k (column k of a periodic record)");
    println!();
    println!("EXAMPLES:");
    println!("    trex scan '<\\W:t>.*</=t>' --text '<div>hi</div>'");
    println!("    trex rewrite '\\E:e' '[redacted]' --text 'ping bob@x.com'");
    println!("    trex rewrite '<\\W:t>(.*):b</=t>' '<${{t}}>${{b:upper}}</${{t}}>' page.html");
    println!("    trex scan '\\W:x =x' --text 'the the cat'");
    println!("    trex scan '\\W\\B(.*)' --text 'call f(g(x))'");
    println!("    trex prefilter access.log --literal ERROR --literal CRITICAL");
    println!("    trex prefilter access.log --verify");
    println!("    trex compress big.log --chunks 0");
    println!("    trex echo server.log --super");
}

/// List the shipped library: each entry's name, whether it is a kind the
/// lexer produces or a sub-pattern the parser inlines, whether a check
/// guards it, and what it matches. With `--test`, run the `test` lines of
/// the pattern files named instead, or the shipped library's own where no
/// file is named.
fn run_lib(args: &[String]) -> ExitCode {
    let mut json = false;
    let mut test = false;
    let mut tests: Vec<String> = Vec::new();
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            "--test" => test = true,
            bad if bad.starts_with('-') => {
                eprintln!("trex lib: unknown argument {bad}");
                return ExitCode::FAILURE;
            }
            path => tests.push(path.to_string()),
        }
    }
    if !test && !tests.is_empty() {
        eprintln!("trex lib: {} names a pattern file; --test runs its test lines", tests[0]);
        return ExitCode::FAILURE;
    }
    if test {
        if json {
            eprintln!("trex lib: --test prints each failure as a file:line: line and takes no --json");
            return ExitCode::FAILURE;
        }
        return if tests.is_empty() { run_library_tests() } else { run_lib_tests(&tests) };
    }
    let entries = trex::library::entries();
    if json {
        let mut out = String::from("[");
        for (i, e) in entries.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&format!(
                "{{\"name\":\"{}\",\"form\":\"{}\",\"guarded\":{},\"what\":\"{}\"}}",
                json_escape(e.name),
                if e.is_kind() { "kind" } else { "pattern" },
                e.is_guarded(),
                json_escape(e.what)
            ));
        }
        out.push(']');
        println!("{out}");
        return ExitCode::SUCCESS;
    }
    println!("{:<14} {:<8} {:<8} what", "name", "form", "guard");
    for e in entries {
        let guard = if e.reads_context() {
            "context"
        } else if e.is_guarded() {
            "checked"
        } else {
            "shape"
        };
        println!(
            "{:<14} {:<8} {:<8} {}",
            e.name,
            if e.is_kind() { "kind" } else { "pattern" },
            guard,
            e.what
        );
    }
    ExitCode::SUCCESS
}

/// Run the shipped library's own `test` lines, through the parser and the
/// checker a user's file goes through, and report as that report reads. What
/// it answers is whether this build's kinds and their checks behave, which a
/// reader of a binary they did not build cannot ask any other way.
fn run_library_tests() -> ExitCode {
    let mut shapes = trex::ShapeSet::new();
    for (n, line) in trex::library::TESTS.iter().enumerate() {
        if let Err(e) = shapes.declare_test(line, n + 1) {
            eprintln!("trex lib: the library's own test line {}: {}", n + 1, e.msg);
            return ExitCode::FAILURE;
        }
    }
    let failures = shapes.run_tests();
    for failure in &failures {
        println!("{failure}");
    }
    let total = shapes.tests().len();
    let failed: std::collections::BTreeSet<usize> = failures.iter().map(|f| f.line).collect();
    let plural = if total == 1 { "" } else { "s" };
    if failed.is_empty() {
        println!("the library: {total} test{plural} passed");
        ExitCode::SUCCESS
    } else {
        println!("the library: {} of {total} test{plural} failed", failed.len());
        ExitCode::FAILURE
    }
}

/// Declare the pattern files in order into one set, as repeated `--lib`
/// flags do, and run every `test` line they hold against what the set
/// finally declares. Each failure prints as a `FILE:LINE:` line, then one
/// line per file counts its tests; the status is failure when any test is.
fn run_lib_tests(paths: &[String]) -> ExitCode {
    let mut shapes = trex::ShapeSet::new();
    for path in paths {
        if let Err(e) = shapes.declare_file(std::path::Path::new(path)) {
            eprintln!("trex lib: {}", e.msg);
            return ExitCode::FAILURE;
        }
    }
    let failures = shapes.run_tests();
    for failure in &failures {
        println!("{failure}");
    }
    let mut any_failed = false;
    for path in paths {
        let file = std::path::Path::new(path);
        let total = shapes.tests().iter().filter(|t| t.file.as_deref() == Some(file)).count();
        let failed: std::collections::BTreeSet<usize> =
            failures.iter().filter(|f| f.file.as_deref() == Some(file)).map(|f| f.line).collect();
        let plural = if total == 1 { "" } else { "s" };
        if total == 0 {
            println!("{path}: no test lines");
        } else if failed.is_empty() {
            println!("{path}: {total} test{plural} passed");
        } else {
            println!("{path}: {} of {total} test{plural} failed", failed.len());
            any_failed = true;
        }
    }
    if any_failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

fn run_grammar(args: &[String]) -> ExitCode {
    if args.is_empty() {
        eprintln!("usage: trex grammar GRAMMAR (FILE | --text STRING) [--grammar-text SRC] [--start RULE] [--count | --best | --prob]");
        return ExitCode::FAILURE;
    }
    let mut grammar_src: Option<String> = None;
    let mut input: Option<Vec<u8>> = None;
    let mut start: Option<String> = None;
    let mut count = false;
    let mut best = false;
    let mut prob = false;
    let mut segment: Option<String> = None;
    let mut dict_path: Option<String> = None;

    // The first positional is the grammar file unless --grammar-text is
    // given.
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--grammar-text" => {
                i += 1;
                grammar_src = args.get(i).cloned();
            }
            "--text" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --text needs a value");
                    return ExitCode::FAILURE;
                };
                input = Some(v.clone().into_bytes());
            }
            "--start" => {
                i += 1;
                start = args.get(i).cloned();
            }
            "--count" => count = true,
            "--best" => best = true,
            "--prob" => prob = true,
            "--segment" => {
                i += 1;
                segment = args.get(i).cloned();
            }
            "--dict" => {
                i += 1;
                dict_path = args.get(i).cloned();
            }
            path if grammar_src.is_none() => match std::fs::read_to_string(path) {
                Ok(s) => grammar_src = Some(s),
                Err(e) => {
                    eprintln!("trex: cannot read grammar {path}: {e}");
                    return ExitCode::FAILURE;
                }
            },
            path => match read_input(path) {
                Ok(b) => input = Some(b),
                Err(e) => {
                    eprintln!("trex: cannot read {path}: {e}");
                    return ExitCode::FAILURE;
                }
            },
        }
        i += 1;
    }

    let Some(grammar_src) = grammar_src else {
        eprintln!("trex grammar: no grammar given (pass a GRAMMAR file or --grammar-text SRC)");
        return ExitCode::FAILURE;
    };
    let mut grammar = match trex::Grammar::parse(&grammar_src) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("trex: grammar error at line {}: {}", e.line, e.msg);
            return ExitCode::FAILURE;
        }
    };
    if let Some(s) = start {
        grammar = grammar.with_start(&s);
    }

    // Segmentation lattice: parse a run-together string over a dictionary,
    // so the grammar disambiguates the tokenization. --count reports the
    // segmentations the grammar accepts, --best the most probable.
    if let Some(seg) = segment {
        let Some(dpath) = dict_path else {
            eprintln!("trex grammar: --segment needs --dict FILE");
            return ExitCode::FAILURE;
        };
        let dict_src = match std::fs::read_to_string(&dpath) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("trex: cannot read dict {dpath}: {e}");
                return ExitCode::FAILURE;
            }
        };
        let dict: Vec<String> = dict_src.split_whitespace().map(String::from).collect();
        if best {
            let p = grammar.best_segmentation_prob(&seg, &dict);
            println!("{p}");
            return if p > 0.0 { ExitCode::SUCCESS } else { ExitCode::FAILURE };
        }
        let n = grammar.count_segmentations(&seg, &dict);
        println!("{n}");
        return if n > 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE };
    }

    let Some(input) = input else {
        eprintln!("trex grammar: no input given (pass a FILE or --text STRING)");
        return ExitCode::FAILURE;
    };

    // The semiring surfaces report a value over every derivation, which the
    // single ordered-choice parse below cannot show: `--count` the number of
    // derivations, `--best` the most probable derivation's probability under
    // the grammar's `@p` weights, `--prob` the total probability.
    if count {
        let n = grammar.count_parses(&input);
        println!("{n}");
        return if n > 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE };
    }
    if best || prob {
        let p = if best { grammar.best_parse_prob(&input) } else { grammar.total_prob(&input) };
        println!("{p}");
        return if p > 0.0 { ExitCode::SUCCESS } else { ExitCode::FAILURE };
    }

    match grammar.parse_input(&input) {
        Some(node) => {
            println!("{}", node.sexpr());
            ExitCode::SUCCESS
        }
        None => {
            println!("no parse");
            ExitCode::FAILURE
        }
    }
}

fn run_bpe(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("train") => run_bpe_train(&args[1..]),
        Some("encode") => run_bpe_encode(&args[1..]),
        _ => {
            eprintln!("usage: trex bpe (train CORPUS [--merges K] [--max-bytes N] | encode (--text STRING | FILE) --model MODEL)");
            ExitCode::FAILURE
        }
    }
}

fn run_bpe_train(args: &[String]) -> ExitCode {
    let mut corpus_path: Option<String> = None;
    let mut merges = 1000usize;
    let mut max_bytes: Option<usize> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--merges" => {
                i += 1;
                merges = flag_value_or_fail!("bpe train", args, i, "--merges");
            }
            "--max-bytes" => {
                i += 1;
                max_bytes = Some(flag_value_or_fail!("bpe train", args, i, "--max-bytes"));
            }
            path if corpus_path.is_none() => corpus_path = Some(path.to_string()),
            other => {
                eprintln!("trex bpe train: unexpected argument {other}");
                return ExitCode::FAILURE;
            }
        }
        i += 1;
    }
    let Some(path) = corpus_path else {
        eprintln!("trex bpe train: no corpus given");
        return ExitCode::FAILURE;
    };
    let corpus = match read_input(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("trex: cannot read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let bytes = match max_bytes {
        Some(cap) => trex::bpe::sample(&corpus, cap),
        None => &corpus[..],
    };
    let t = std::time::Instant::now();
    let bpe = trex::bpe::Bpe::train(bytes, merges);
    eprintln!(
        "trex bpe: learned {} merges from {} bytes in {:.1}s",
        bpe.len(),
        bytes.len(),
        t.elapsed().as_secs_f64()
    );
    print!("{}", bpe.to_lines());
    ExitCode::SUCCESS
}

fn run_bpe_encode(args: &[String]) -> ExitCode {
    let mut text: Option<Vec<u8>> = None;
    let mut model_path: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--text" => {
                i += 1;
                text = args.get(i).map(|s| s.clone().into_bytes());
            }
            "--model" => {
                i += 1;
                model_path = args.get(i).cloned();
            }
            path if text.is_none() => match std::fs::read(path) {
                Ok(b) => text = Some(b),
                Err(e) => {
                    eprintln!("trex: cannot read {path}: {e}");
                    return ExitCode::FAILURE;
                }
            },
            other => {
                eprintln!("trex bpe encode: unexpected argument {other}");
                return ExitCode::FAILURE;
            }
        }
        i += 1;
    }
    let (Some(text), Some(model_path)) = (text, model_path) else {
        eprintln!("trex bpe encode: needs (--text STRING | FILE) and --model MODEL");
        return ExitCode::FAILURE;
    };
    let model = match std::fs::read_to_string(&model_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("trex: cannot read model {model_path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let bpe = match trex::bpe::Bpe::parse(&model) {
        Ok(bpe) => bpe,
        Err(e) => {
            eprintln!("trex bpe encode: {model_path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("{}", bpe.encode(&text).join(" "));
    ExitCode::SUCCESS
}

fn run_observe(args: &[String]) -> ExitCode {
    let mut input: Option<Vec<u8>> = None;
    let mut show_field = false;
    let mut show_contested = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--field" => show_field = true,
            "--contested" => show_contested = true,
            "--text" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --text needs a value");
                    return ExitCode::FAILURE;
                };
                input = Some(v.clone().into_bytes());
            }
            path => match read_input(path) {
                Ok(b) => input = Some(b),
                Err(e) => {
                    eprintln!("trex: cannot read {path}: {e}");
                    return ExitCode::FAILURE;
                }
            },
        }
        i += 1;
    }
    let Some(input) = input else {
        eprintln!("trex observe: no input given (pass a FILE or --text STRING)");
        return ExitCode::FAILURE;
    };
    use trex::observation;
    let f = observation::analyze(&input);
    println!(
        "trex observe: {} bytes, {} contested point(s)",
        input.len(),
        f.contested.len()
    );
    if let Some(&idx) = f.contested.iter().max_by(|&&a, &&b| {
        f.frames[a]
            .disagreement
            .partial_cmp(&f.frames[b].disagreement)
            .unwrap_or(std::cmp::Ordering::Equal)
    }) {
        println!(
            "  peak observer-dependence: {:.2} at byte {idx}",
            f.frames[idx].disagreement
        );
    }
    if show_contested {
        println!("  contested points ({}):", f.contested.len());
        for &b in &f.contested {
            let lo = b.saturating_sub(10);
            let preview = String::from_utf8_lossy(&input[lo..(b + 14).min(input.len())])
                .replace(['\n', '\t'], " ");
            println!("    @{b:>6}  ...{preview}...");
        }
    }
    if show_field {
        let n = f.frames.len();
        let hop = (n / 24).max(1);
        println!("  vantage (causal | anticausal | centered | disagree), every {hop} bytes:");
        let mut t = 0;
        while t < n {
            let fr = &f.frames[t];
            println!(
                "    @{t:>6}  c={:.2} a={:.2} m={:.2} |d|={:.2}",
                fr.causal, fr.anticausal, fr.centered, fr.disagreement
            );
            t += hop;
        }
    }
    ExitCode::SUCCESS
}

fn run_flow(args: &[String]) -> ExitCode {
    let mut input: Option<Vec<u8>> = None;
    let mut over = trex::flow::Signal::Magnitude;
    let mut show_field = false;
    let mut show_reversals = false;
    let mut limit = usize::MAX;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--field" => show_field = true,
            "--reversals" => show_reversals = true,
            "--limit" => {
                i += 1;
                limit = flag_value_or_fail!("flow", args, i, "--limit");
            }
            "--over" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --over needs a signal (magnitude|stress|length)");
                    return ExitCode::FAILURE;
                };
                match trex::flow::Signal::parse(v) {
                    Some(s) => over = s,
                    None => {
                        eprintln!("trex: unknown flow signal '{v}' (magnitude|stress|length)");
                        return ExitCode::FAILURE;
                    }
                }
            }
            "--text" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --text needs a value");
                    return ExitCode::FAILURE;
                };
                input = Some(v.clone().into_bytes());
            }
            path => match read_input(path) {
                Ok(b) => input = Some(b),
                Err(e) => {
                    eprintln!("trex: cannot read {path}: {e}");
                    return ExitCode::FAILURE;
                }
            },
        }
        i += 1;
    }
    let Some(input) = input else {
        eprintln!("trex flow: no input given (pass a FILE or --text STRING)");
        return ExitCode::FAILURE;
    };
    use trex::flow;
    let f = flow::analyze_bytes(&input, over);
    println!(
        "trex flow (over {}): {} bytes, {} tokens, {} reversal(s)",
        over.label(),
        input.len(),
        f.n_tokens,
        f.reversals.len()
    );
    if let Some(fr) = f.frames.iter().max_by_key(|fr| fr.momentum) {
        let dir = match fr.direction {
            1 => "rising",
            -1 => "falling",
            _ => "steady",
        };
        println!("  peak momentum: {} ({dir})", fr.momentum);
    }
    if show_field {
        println!("  per-token (token -> slope | direction | momentum):");
        for (k, fr) in f.frames.iter().enumerate().take(limit) {
            let (s, e) = f.spans[k];
            let w = String::from_utf8_lossy(&input[s..e]).replace('\n', "\\n");
            let arrow = match fr.direction {
                1 => "up",
                -1 => "down",
                _ => "--",
            };
            println!(
                "    {:<14} slope={:>+7.2} {:<4} mom={}",
                format!("{w:.14}"),
                fr.slope,
                arrow,
                fr.momentum
            );
        }
    }
    if show_reversals {
        println!("  reversals ({}):", f.reversals.len());
        for &b in &f.reversals {
            let preview =
                String::from_utf8_lossy(&input[b..(b + 24).min(input.len())]).replace('\n', " ");
            println!("    @{b:>6}  {preview}");
        }
    }
    ExitCode::SUCCESS
}

fn run_echo(args: &[String]) -> ExitCode {
    let mut input: Option<Vec<u8>> = None;
    let mut show_field = false;
    let mut show_super = false;
    let mut orbit = trex::OrbitGroup::Identity;
    let mut top = 8usize;
    // --field prints the field entire; --limit N bounds it. A bound is opt-in,
    // never baked in: a silent cap on an analysis dump is a lie by omission.
    let mut limit = usize::MAX;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--field" => show_field = true,
            "--super" => show_super = true,
            "--top" => {
                i += 1;
                top = flag_value_or_fail!("echo", args, i, "--top");
            }
            "--limit" => {
                i += 1;
                limit = flag_value_or_fail!("echo", args, i, "--limit");
            }
            "--orbit" => {
                i += 1;
                orbit = match args.get(i).map(String::as_str) {
                    None => trex::OrbitGroup::Identity,
                    Some(name) => match trex::OrbitGroup::parse(name) {
                        Some(g) => g,
                        None => {
                            eprintln!(
                                "trex echo: unknown orbit '{name}' (use identity, case, shape, notation, e8, ip, url, time, path, fold, numeric, or a typed relation such as subnet/24, domain or day)"
                            );
                            return ExitCode::FAILURE;
                        }
                    },
                };
            }
            "--text" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --text needs a value");
                    return ExitCode::FAILURE;
                };
                input = Some(v.clone().into_bytes());
            }
            path => match read_input(path) {
                Ok(b) => input = Some(b),
                Err(e) => {
                    eprintln!("trex: cannot read {path}: {e}");
                    return ExitCode::FAILURE;
                }
            },
        }
        i += 1;
    }
    let Some(input) = input else {
        eprintln!("trex echo: no input given (pass a FILE or --text STRING)");
        return ExitCode::FAILURE;
    };
    use trex::echo;
    let toks = trex::lexer::lex(&input);
    let cfg = echo::EchoConfig { orbit, ..Default::default() };
    let f = echo::analyze_with(&toks, &input, &cfg);
    println!(
        "trex echo: {} bytes, {} tokens ({} keyed, {} distinct), novelty {:.1}%, echo rate {:.1}%",
        input.len(),
        toks.len(),
        f.keyed,
        f.distinct,
        f.novelty() * 100.0,
        f.echo_rate() * 100.0
    );
    // The strongest echoes: one line per recurring key, most occurrences first.
    let mut best: Vec<&echo::EchoFrame> = f
        .frames
        .iter()
        .filter(|fr| fr.echoed() && fr.back_lag.is_none())
        .collect();
    best.sort_by(|a, b| b.count.cmp(&a.count).then(a.start.cmp(&b.start)));
    if !best.is_empty() {
        println!("  strongest echoes (top {} of {}; first occurrence, count, period):", top.min(best.len()), best.len());
        for fr in best.iter().take(top) {
            let w = String::from_utf8_lossy(&input[fr.start as usize..fr.end as usize])
                .replace('\n', "\\n");
            if fr.period > 0.0 {
                let p = fr.period;
                println!("    {:<20} x{:<5} period ~{p:.0} B", format!("{w:.20}"), fr.count);
            } else {
                println!("    {:<20} x{:<5}", format!("{w:.20}"), fr.count);
            }
        }
    }
    if show_field {
        println!("  per-token (token -> count | back-lag | novel):");
        for fr in f.frames.iter().filter(|fr| fr.keyed).take(limit) {
            let w = String::from_utf8_lossy(&input[fr.start as usize..fr.end as usize])
                .replace('\n', "\\n");
            let lag = fr.back_lag.map_or("-".to_string(), |l| l.to_string());
            println!(
                "    {:<20} x{:<4} back={:>7} {}",
                format!("{w:.20}"),
                fr.count,
                lag,
                if fr.novel() { "novel" } else { "" }
            );
        }
    }
    if show_super {
        let rhymes = echo::analyze_super(&input);
        println!(
            "  structural rhyme (top {} of {} recurring supertoken structures):",
            top.min(rhymes.len()),
            rhymes.len()
        );
        for r in rhymes.iter().take(top) {
            match r.period {
                Some(p) => println!("    {:<28} x{:<4} period ~{p:.0} B", format!("{:.28}", r.key), r.count),
                None => println!("    {:<28} x{:<4}", format!("{:.28}", r.key), r.count),
            }
        }
    }
    ExitCode::SUCCESS
}

fn run_relation(args: &[String]) -> ExitCode {
    let mut input: Option<Vec<u8>> = None;
    let mut show_edges = false;
    let mut show_field = false;
    let mut show_gauge = false;
    let mut limit = usize::MAX;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--edges" => show_edges = true,
            "--field" => show_field = true,
            "--gauge" => show_gauge = true,
            "--limit" => {
                i += 1;
                limit = flag_value_or_fail!("relation", args, i, "--limit");
            }
            "--text" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --text needs a value");
                    return ExitCode::FAILURE;
                };
                input = Some(v.clone().into_bytes());
            }
            path => match read_input(path) {
                Ok(b) => input = Some(b),
                Err(e) => {
                    eprintln!("trex: cannot read {path}: {e}");
                    return ExitCode::FAILURE;
                }
            },
        }
        i += 1;
    }
    let Some(input) = input else {
        eprintln!("trex relation: no input given (pass a FILE or --text STRING)");
        return ExitCode::FAILURE;
    };
    use trex::relation::{self, RelationKind};
    let f = relation::analyze_bytes(&input);
    let count = |k: RelationKind| f.edges_of(k).count();
    println!(
        "trex relation: {} bytes, {} tokens, max depth {}, nesting load {}",
        input.len(),
        f.n_tokens,
        f.max_depth(),
        f.nesting_load
    );
    println!(
        "  edges: {} encloses, {} operator, {} adjacent",
        count(RelationKind::Encloses),
        count(RelationKind::Operator),
        count(RelationKind::Adjacent)
    );
    println!(
        "  holonomy: {} ({} reuse chord(s), {} scope-crossing), net {:+} ({})",
        f.holonomy(),
        f.chords.len(),
        f.twisted_chords(),
        f.net_holonomy(),
        match f.net_holonomy().signum() {
            1 => "reuse flows inward",
            -1 => "reuse flows outward",
            _ => "balanced",
        }
    );
    let h = trex::holography::analyze_bytes(&input);
    let rebuilt = h.reconstruct_bulk(&trex::lexer::lex(&input));
    println!(
        "  holography: {} boundary events, {} bulk node(s), holographic defect {} (= reuse chords){}",
        h.boundary.len(),
        h.bulk_nodes,
        h.holographic_defect,
        if h.boundary_closed { "" } else { "; boundary NOT closed" }
    );
    println!(
        "  bulk = {} enclosure edge(s) rebuilt from the boundary + {} holonomy chord(s)",
        rebuilt.len(),
        f.chords.len()
    );
    let curv = trex::curvature::analyze_bytes(&input);
    println!(
        "  curvature: min {} (sharpest bottleneck), mean {:.2}, {} bridge edge(s)",
        curv.min_ricci(),
        curv.mean_ricci(),
        curv.bridges()
    );
    let topo = trex::topology::analyze_bytes(&input);
    println!(
        "  topology: {} nodes, {} edges, b0 {} component(s), b1 {} independent loop(s), euler {}",
        topo.nodes,
        topo.edges,
        topo.components,
        topo.cycle_rank,
        topo.euler()
    );
    let geo = trex::geodesic::analyze_bytes(&input);
    println!(
        "  geodesic: longest reuse shortcut collapses a {}-token span to one hop",
        geo.max_shortcut()
    );
    let ent = trex::entanglement::analyze_bytes(&input);
    match ent.min_cut() {
        Some((k, c)) => println!(
            "  entanglement: peak {}, minimal cut {} crossing(s) before token {k}",
            ent.max_entanglement(),
            c
        ),
        None => println!("  entanglement: peak {}", ent.max_entanglement()),
    }
    if show_gauge {
        println!("  gauge-fixed (alpha-equivalence canonical form):");
        println!("    {}", trex::gauge::canonicalize_bytes(&input));
    }
    let word = |i: usize| String::from_utf8_lossy(&input[f.spans[i].0..f.spans[i].1]).into_owned();
    if show_edges {
        let named = |k: RelationKind, label: &str| {
            for e in f.edges_of(k).take(limit) {
                println!("    {label:<9} {:>10} -> {}", word(e.from), word(e.to));
            }
        };
        println!("  directed relations (from -> to):");
        named(RelationKind::Encloses, "encloses");
        named(RelationKind::Operator, "operator");
        named(RelationKind::Adjacent, "adjacent");
        if !f.chords.is_empty() {
            println!("  reuse chords (from -> to, residual = scope jump):");
            for c in f.chords.iter().take(limit) {
                println!("    {:>10} -> {:<10} residual {:+}", word(c.from), word(c.to), c.residual);
            }
        }
    }
    if show_field {
        println!("  per-token (token -> depth | enclosing heads, outer first):");
        for (i, fr) in f.frames.iter().enumerate().take(limit) {
            if fr.depth == 0 {
                continue;
            }
            let heads: Vec<String> = fr.enclosure.iter().map(|&h| word(h)).collect();
            println!("    {:<14} d={:>2}  [{}]", format!("{:.14}", word(i)), fr.depth, heads.join(" > "));
        }
    }
    ExitCode::SUCCESS
}

fn run_stress(args: &[String]) -> ExitCode {
    let mut input: Option<Vec<u8>> = None;
    let mut show_field = false;
    let mut show_peaks = false;
    let mut show_fractures = false;
    let mut limit = usize::MAX;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--field" => show_field = true,
            "--peaks" => show_peaks = true,
            "--fractures" => show_fractures = true,
            "--limit" => {
                i += 1;
                limit = flag_value_or_fail!("stress", args, i, "--limit");
            }
            "--text" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --text needs a value");
                    return ExitCode::FAILURE;
                };
                input = Some(v.clone().into_bytes());
            }
            path => match read_input(path) {
                Ok(b) => input = Some(b),
                Err(e) => {
                    eprintln!("trex: cannot read {path}: {e}");
                    return ExitCode::FAILURE;
                }
            },
        }
        i += 1;
    }
    let Some(input) = input else {
        eprintln!("trex stress: no input given (pass a FILE or --text STRING)");
        return ExitCode::FAILURE;
    };
    use trex::stress;
    let f = stress::analyze_bytes(&input);
    println!(
        "trex stress: {} bytes, {} tokens, max depth {}, {} peak(s), {} fracture(s)",
        input.len(),
        f.n_tokens,
        f.max_depth,
        f.peaks.len(),
        f.fractures.len()
    );
    if let Some(fr) = f
        .frames
        .iter()
        .max_by(|a, b| a.load.partial_cmp(&b.load).unwrap_or(std::cmp::Ordering::Equal))
    {
        println!(
            "  peak load: {:.0} (depth {}, strain {:.0})",
            fr.load, fr.depth, fr.strain
        );
    }
    if show_field {
        println!("  per-token (token -> depth | strain | load):");
        for (k, fr) in f.frames.iter().enumerate().take(limit) {
            let (s, e) = f.spans[k];
            let w = String::from_utf8_lossy(&input[s..e]).replace('\n', "\\n");
            println!(
                "    {:<14} d={:>3} strain={:>5.0} load={:>6.0}",
                format!("{w:.14}"),
                fr.depth,
                fr.strain,
                fr.load
            );
        }
    }
    if show_peaks {
        println!("  stress peaks ({}):", f.peaks.len());
        for &b in &f.peaks {
            let preview =
                String::from_utf8_lossy(&input[b..(b + 24).min(input.len())]).replace('\n', " ");
            println!("    @{b:>6}  {preview}");
        }
    }
    if show_fractures {
        println!("  fractures ({}):", f.fractures.len());
        for &b in &f.fractures {
            let preview =
                String::from_utf8_lossy(&input[b..(b + 24).min(input.len())]).replace('\n', " ");
            println!("    @{b:>6}  {preview}");
        }
    }
    ExitCode::SUCCESS
}

fn run_magnitude(args: &[String]) -> ExitCode {
    let mut input: Option<Vec<u8>> = None;
    let mut show_field = false;
    let mut show_jumps = false;
    let mut show_energy = false;
    let mut show_outliers = false;
    let mut limit = usize::MAX;
    let mut top = 3usize;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--field" => show_field = true,
            "--jumps" => show_jumps = true,
            "--energy" => show_energy = true,
            "--outliers" => show_outliers = true,
            "--limit" => {
                i += 1;
                limit = flag_value_or_fail!("magnitude", args, i, "--limit");
            }
            "--top" => {
                i += 1;
                top = flag_value_or_fail!("magnitude", args, i, "--top");
            }
            "--text" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --text needs a value");
                    return ExitCode::FAILURE;
                };
                input = Some(v.clone().into_bytes());
            }
            path => match read_input(path) {
                Ok(b) => input = Some(b),
                Err(e) => {
                    eprintln!("trex: cannot read {path}: {e}");
                    return ExitCode::FAILURE;
                }
            },
        }
        i += 1;
    }
    let Some(input) = input else {
        eprintln!("trex magnitude: no input given (pass a FILE or --text STRING)");
        return ExitCode::FAILURE;
    };
    use trex::magnitude;
    let f = magnitude::analyze_bytes(&input);
    let cmp = |a: &f32, b: &f32| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal);
    println!(
        "trex magnitude: {} bytes, {} tokens, total energy {:.1}, {} scale-jump(s)",
        input.len(),
        f.n_tokens,
        f.total_energy,
        f.jumps.len()
    );
    if let Some((idx, fr)) = f
        .frames
        .iter()
        .enumerate()
        .max_by(|a, b| cmp(&a.1.magnitude, &b.1.magnitude))
    {
        let (s, e) = f.spans[idx];
        let tok = String::from_utf8_lossy(&input[s..e]);
        println!("  peak magnitude: {:.2} at '{}'", fr.magnitude, tok.trim());
    }
    if show_field {
        println!("  per-token (token -> magnitude | gradient | energy):");
        for (k, fr) in f.frames.iter().enumerate().take(limit) {
            let (s, e) = f.spans[k];
            let w = String::from_utf8_lossy(&input[s..e]).replace('\n', "\\n");
            println!(
                "    {:<14} m={:>6.2} dm={:>+6.2} E={:>7.1}",
                format!("{w:.14}"),
                fr.magnitude,
                fr.gradient,
                fr.energy
            );
        }
    }
    if show_jumps {
        println!("  scale discontinuities ({} jump(s)):", f.jumps.len());
        for &b in &f.jumps {
            let preview =
                String::from_utf8_lossy(&input[b..(b + 24).min(input.len())]).replace('\n', " ");
            println!("    @{b:>6}  {preview}");
        }
    }
    if show_outliers {
        let o = f.outliers();
        println!("  scale outliers ({}):", o.len());
        for idx in o {
            let (s, e) = f.spans[idx];
            let tok = String::from_utf8_lossy(&input[s..e]);
            println!("    '{}'  m={:.2}", tok.trim(), f.frames[idx].magnitude);
        }
    }
    if show_energy {
        let mut idx: Vec<usize> = (0..f.frames.len()).collect();
        idx.sort_by(|&a, &b| cmp(&f.frames[b].energy, &f.frames[a].energy));
        // A ranked highlight, so the bound is explicit in the header and
        // user-set (--top); the total is printed so nothing hides.
        println!("  heaviest points (top {} of {} by local energy):", top.min(idx.len()), idx.len());
        for &k in idx.iter().take(top) {
            let (s, e) = f.spans[k];
            let tok = String::from_utf8_lossy(&input[s..e]);
            println!("    '{}'  E={:.1}", tok.trim(), f.frames[k].energy);
        }
    }
    ExitCode::SUCCESS
}

fn run_shape(args: &[String]) -> ExitCode {
    let mut input: Option<Vec<u8>> = None;
    let mut classes = false;
    let mut period = false;
    let mut segment = false;
    let mut json = false;
    let mut orbit_group: Option<trex::orbit::OrbitGroup> = None;
    let mut limit = usize::MAX;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--classes" => classes = true,
            "--period" => period = true,
            "--segment" => segment = true,
            "--json" => json = true,
            "--limit" => {
                i += 1;
                limit = flag_value_or_fail!("shape", args, i, "--limit");
            }
            "--orbit" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --orbit needs a group (identity|case|notation|shape|e8|ip|url|time|path|fold|numeric, or a typed relation such as subnet/24)");
                    return ExitCode::FAILURE;
                };
                match trex::orbit::OrbitGroup::parse(v) {
                    Some(g) => orbit_group = Some(g),
                    None => {
                        eprintln!("trex: unknown orbit group '{v}' (identity|case|notation|shape|e8|ip|url|time|path|fold|numeric, or a typed relation such as subnet/24)");
                        return ExitCode::FAILURE;
                    }
                }
            }
            "--text" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --text needs a value");
                    return ExitCode::FAILURE;
                };
                input = Some(v.clone().into_bytes());
            }
            path => match read_input(path) {
                Ok(b) => input = Some(b),
                Err(e) => {
                    eprintln!("trex: cannot read {path}: {e}");
                    return ExitCode::FAILURE;
                }
            },
        }
        i += 1;
    }
    let Some(input) = input else {
        eprintln!("trex shape: no input given (pass a FILE or --text STRING)");
        return ExitCode::FAILURE;
    };
    use trex::shape;
    let field = match orbit_group {
        Some(g) => shape::analyze_bytes_over(&input, g),
        None => shape::analyze_bytes(&input),
    };
    let regions = field.shape_regions();

    if json {
        print!("{{\"n_tokens\":{},\"boundaries\":[", field.n_tokens);
        for (k, b) in field.boundaries.iter().enumerate() {
            if k > 0 {
                print!(",");
            }
            print!("{b}");
        }
        print!("],\"regions\":[");
        for (k, (s, e, p)) in regions.iter().enumerate() {
            if k > 0 {
                print!(",");
            }
            print!("{{\"start\":{s},\"end\":{e},\"period\":{p}}}");
        }
        println!("]}}");
        return ExitCode::SUCCESS;
    }

    let dom = field
        .frames
        .iter()
        .filter(|f| f.period > 0)
        .max_by(|a, b| {
            a.period_strength
                .partial_cmp(&b.period_strength)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|f| (f.period, f.period_strength));
    println!(
        "trex shape: {} bytes, {} tokens, {} template region(s), {} change-point(s)",
        input.len(),
        field.n_tokens,
        regions.len(),
        field.boundaries.len()
    );
    match dom {
        Some((p, s)) => println!("  dominant shape-period: {p} tokens  (strength {s:.2})"),
        None => println!("  dominant shape-period: none (no structural repetition)"),
    }

    if period {
        println!("  template regions (byte span -> shape-period):");
        for (s, e, p) in &regions {
            let preview =
                String::from_utf8_lossy(&input[*s..(*s + 28).min(*e)]).replace(['\n', '\t'], " ");
            println!("    [{s:>8}..{e:<8}] period {p:<4} {preview}");
        }
    }
    if segment {
        let b: Vec<String> = field.boundaries.iter().map(usize::to_string).collect();
        println!("  shape change-points ({}):  {}", b.len(), b.join(" "));
    }
    if classes {
        println!("  per-token (token -> class | period | novelty):");
        for (k, f) in field.frames.iter().enumerate().take(limit) {
            let (s, e) = field.spans[k];
            let w = String::from_utf8_lossy(&input[s..e]).replace('\n', "\\n");
            println!(
                "    {:<14} cls={:08x} per={:<3} nov={:.2}",
                format!("{w:.14}"),
                f.class,
                f.period,
                f.novelty
            );
        }
    }
    ExitCode::SUCCESS
}

fn run_spectral(args: &[String]) -> ExitCode {
    let mut input: Option<Vec<u8>> = None;
    let mut segment = false;
    let mut bands = false;
    let mut classify = false;
    let mut code_classify = false;
    let mut json = false;
    let mut limit = usize::MAX;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--segment" => segment = true,
            "--bands" => bands = true,
            "--classify" => classify = true,
            "--code-classify" => code_classify = true,
            "--json" => json = true,
            "--limit" => {
                i += 1;
                limit = flag_value_or_fail!("spectral", args, i, "--limit");
            }
            "--text" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --text needs a value");
                    return ExitCode::FAILURE;
                };
                input = Some(v.clone().into_bytes());
            }
            path => match read_input(path) {
                Ok(b) => input = Some(b),
                Err(e) => {
                    eprintln!("trex: cannot read {path}: {e}");
                    return ExitCode::FAILURE;
                }
            },
        }
        i += 1;
    }
    let Some(input) = input else {
        eprintln!("trex spectral: no input given (pass a FILE or --text STRING)");
        return ExitCode::FAILURE;
    };

    use trex::spectral;
    // The code-texture axis: split the input at spectral change-points and label
    // each region by its code-tuned texture (code / blob / prose / numeric) - the
    // grammar-free region map the construct / disasm layers read, each region
    // textured from a fresh pass over its own bytes so the label does not bleed.
    if code_classify {
        // Fused region classification: spectral texture x shape period. A
        // tabular block reads as one `table` region (the shape axis), where the
        // byte-period alone reads only "data".
        let regions = trex::shape::classified_regions(&input);
        println!(
            "trex spectral (region classification: shape x spectral): {} bytes, {} regions",
            input.len(),
            regions.len()
        );
        for (s, e, kind) in &regions {
            let preview =
                String::from_utf8_lossy(&input[*s..(*s + 28).min(*e)]).replace(['\n', '\t'], " ");
            let label = match kind {
                trex::shape::RegionKind::Table(p) => format!("table(p{p})"),
                other => format!("{other:?}").to_lowercase(),
            };
            println!("    [{s:>8}..{e:<8}] {label:<10} {preview}");
        }
        return ExitCode::SUCCESS;
    }
    let field = spectral::analyze(&input);

    if json {
        print_spectral_json(&field);
        return ExitCode::SUCCESS;
    }

    println!(
        "trex spectral: {} bytes, {} frames (hop {}), {} change-points",
        field.len,
        field.frames.len(),
        field.hop,
        field.boundaries.len()
    );

    if !field.frames.is_empty() {
        let (mut emin, mut emax, mut esum) = (1.0f32, 0.0f32, 0.0f32);
        for f in &field.frames {
            emin = emin.min(f.entropy);
            emax = emax.max(f.entropy);
            esum += f.entropy;
        }
        let emean = esum / field.frames.len() as f32;
        println!("  entropy   min {emin:.2}  mean {emean:.2}  max {emax:.2}  (normalized bits/byte)");

        // Distinct dominant periods, strongest first.
        let mut periods: Vec<(u16, f32)> = Vec::new();
        for f in &field.frames {
            if f.period != 0 {
                if let Some(e) = periods.iter_mut().find(|(p, _)| *p == f.period) {
                    e.1 = e.1.max(f.period_strength);
                } else {
                    periods.push((f.period, f.period_strength));
                }
            }
        }
        periods.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        if periods.is_empty() {
            println!("  period    none detected (no strong byte-periodicity)");
        } else {
            let shown: Vec<String> =
                periods.iter().map(|(p, s)| format!("{p}@{s:.2}")).collect();
            println!("  period    {}", shown.join("  "));
        }
    }

    // Texture timeline: the intrinsic region split (the default view).
    if classify || (!segment && !bands) {
        println!("  texture timeline (merged regions, cut at change-points):");
        for (s, e, tex) in spectral::regions(&field) {
            let sig = field.signature(s, e);
            println!(
                "    [{s:>8}..{e:<8}] {:<5}  H={:.2} per={} nov={:.2}",
                tex.label(),
                sig.entropy,
                sig.period,
                sig.novelty
            );
        }
    }

    if segment {
        println!("  change-points ({}):", field.boundaries.len());
        let shown: Vec<String> =
            field.boundaries.iter().take(limit).map(usize::to_string).collect();
        println!("    {}", shown.join(" "));
        if field.boundaries.len() > limit {
            println!("    ... (+{} more; raise --limit)", field.boundaries.len() - limit);
        }
    }

    if bands {
        println!("  per-hop frames (medium-clock mix [digit alpha space punct high]):");
        for (j, f) in field.frames.iter().enumerate().take(limit) {
            let off = ((j + 1) * field.hop).min(field.len).saturating_sub(1);
            let m = f.texture_mix();
            println!(
                "    @{off:>8}  [{:.2} {:.2} {:.2} {:.2} {:.2}]  H={:.2} per={:>3} nov={:.2} {}",
                m[0],
                m[1],
                m[2],
                m[3],
                m[4],
                f.entropy,
                f.period,
                f.novelty,
                spectral::texture_of(f).label()
            );
        }
        if field.frames.len() > 256 {
            println!("    ... (+{} more frames)", field.frames.len() - 256);
        }
    }

    ExitCode::SUCCESS
}

/// Emit a [`trex::spectral::SpectralField`] as a single JSON object.
fn print_spectral_json(field: &trex::spectral::SpectralField) {
    print!("{{\"len\":{},\"hop\":{},\"boundaries\":[", field.len, field.hop);
    for (k, b) in field.boundaries.iter().enumerate() {
        if k > 0 {
            print!(",");
        }
        print!("{b}");
    }
    print!("],\"frames\":[");
    for (k, f) in field.frames.iter().enumerate() {
        if k > 0 {
            print!(",");
        }
        print!(
            "{{\"entropy\":{:.4},\"period\":{},\"period_strength\":{:.4},\"novelty\":{:.4},\"bands\":[",
            f.entropy, f.period, f.period_strength, f.novelty
        );
        for (m, v) in f.bands.iter().enumerate() {
            if m > 0 {
                print!(",");
            }
            print!("{v:.4}");
        }
        print!("]}}");
    }
    println!("]}}");
}

/// What a build without the `compress` feature says when asked for the coder:
/// the command exists only where the coder was compiled in, so this names the
/// flag that builds it rather than failing as an unknown command.
#[cfg(not(feature = "compress"))]
fn compress_absent(what: &str) -> String {
    format!(
        "{what}: this binary was built without the compress feature; \
         build one with: cargo build --release --features compress"
    )
}

/// Print the full coder comparison table - bits/byte and model encode speed for
/// every coder - shared by `seam --compress` and `compress --compare`.
#[cfg(feature = "compress")]
fn print_compress_table(label: &str, bytes: &[u8], warm_seed: &[u8]) {
    use trex::seam;
    let n = bytes.len();
    let nf = n as f64;
    let num_merges = (n / 16).clamp(64, 2000);
    macro_rules! timed {
        ($e:expr) => {{
            let t = std::time::Instant::now();
            let v = $e;
            (v, t.elapsed().as_secs_f64())
        }};
    }
    let ((bstream, bdict), bpe_s) = timed!(seam::bpe_total_bits(bytes, num_merges));
    let bpe_total = bstream + bdict;
    let (o0, o0_s) = timed!(seam::adaptive_byte_bits(bytes, 0, false));
    let (mix4, mix4_s) = timed!(seam::adaptive_byte_bits(bytes, 4, false));
    let (ppm3, ppm3_s) = timed!(seam::ppm_byte_bits(bytes, 3, false));
    let (ppm8, ppm8_s) = timed!(seam::ppm_byte_bits(bytes, 8, false));
    let (ppm16, ppm16_s) = timed!(seam::ppm_byte_bits(bytes, 16, false));
    let (ppm16o, ppm16o_s) = timed!(seam::ppm_byte_bits(bytes, 16, true));
    let (lm, lm_s) = timed!(seam::logistic_mix_bits(bytes, false, &[], None));
    let (lmo, lmo_s) = timed!(seam::logistic_mix_bits(bytes, true, &[], None));
    let (lmb, lmb_s) = timed!(seam::logistic_mix_bits(bytes, true, &[], Some(seam::baked_model())));
    let (lmw, lmw_s) = if warm_seed.is_empty() {
        (lmo, lmo_s)
    } else {
        timed!(seam::logistic_mix_bits(bytes, true, warm_seed, None))
    };
    let (ctw, ctw_s) = timed!(seam::ctw_bits(bytes, 24));
    let bpb = |bits: f64| bits / nf;
    let mbps = |secs: f64| (nf / 1e6) / secs.max(1e-9);
    println!("{label}: {n} bytes (bits/byte, lower is better; MB/s = model encode speed)");
    println!("  raw                            : {:.3}", bpb(nf * 8.0));
    println!("  adaptive order-0               : {:.3}   {:>7.2} MB/s", bpb(o0), mbps(o0_s));
    println!("  multi-scale mix, order-4       : {:.3}   {:>7.2} MB/s", bpb(mix4), mbps(mix4_s));
    println!("  BPE stream alone               : {:.3}   {:>7.2} MB/s   (codebook-free target)", bpb(bstream), mbps(bpe_s));
    println!(
        "  BPE stream + codebook          : {:.3}   {:>7.2} MB/s   (= {:.3} + {:.3}; {num_merges} merges)",
        bpb(bpe_total),
        mbps(bpe_s),
        bpb(bstream),
        bpb(bdict)
    );
    println!("  PPM order-3  (ours)            : {:.3}   {:>7.2} MB/s", bpb(ppm3), mbps(ppm3_s));
    println!("  PPM order-8  (ours)            : {:.3}   {:>7.2} MB/s", bpb(ppm8), mbps(ppm8_s));
    println!("  PPM order-16 (ours)            : {:.3}   {:>7.2} MB/s", bpb(ppm16), mbps(ppm16_s));
    println!("  PPM order-16 + orbit context   : {:.3}   {:>7.2} MB/s   (corpus-free, no codebook)", bpb(ppm16o), mbps(ppm16o_s));
    println!("  logistic mix (ours)            : {:.3}   {:>7.2} MB/s   (PAQ-style, literal orders)", bpb(lm), mbps(lm_s));
    println!("  logistic mix + orbit models    : {:.3}   {:>7.2} MB/s   (+ case/shape quotient contexts)", bpb(lmo), mbps(lmo_s));
    println!("  + baked model (instant)        : {:.3}   {:>7.2} MB/s   (byte n-gram integer prior, no warm)", bpb(lmb), mbps(lmb_s));
    if !warm_seed.is_empty() {
        println!(
            "  + warm seed {:>8}B (prior)  : {:.3}   {:>7.2} MB/s   (E8/online warm-start, shared)",
            warm_seed.len(),
            bpb(lmw),
            mbps(lmw_s)
        );
    }
    println!("  CTW depth-24 (ours)            : {:.3}   {:>7.2} MB/s   (Bayesian context-tree mixture)", bpb(ctw), mbps(ctw_s));
    let best = lmw.min(lmb).min(lmo).min(lm).min(ppm16o).min(ppm16).min(ppm8).min(ctw);
    println!(
        "  best ours vs BPE stream        : {:+.3}   ({})",
        bpb(best) - bpb(bstream),
        if best < bstream { "ours wins" } else { "BPE stream wins" }
    );
    println!(
        "  best ours vs BPE stream+codebk : {:+.3}   ({})",
        bpb(best) - bpb(bpe_total),
        if best < bpe_total { "ours wins" } else { "BPE wins" }
    );
}

/// The top-level `compress` command in a build without the `compress` feature.
#[cfg(not(feature = "compress"))]
fn run_compress(_args: &[String]) -> ExitCode {
    eprintln!("{}", compress_absent("trex compress"));
    ExitCode::FAILURE
}

/// The top-level `compress` command: front the coder, report the achieved
/// compression (size, ratio, bits/byte) with the default logistic-mix + orbit
/// coder auto-using the baked prior. `--compare` prints the full coder table.
#[cfg(feature = "compress")]
fn run_compress(args: &[String]) -> ExitCode {
    use trex::seam;
    let mut file: Option<String> = None;
    let mut text: Option<String> = None;
    let mut compare = false;
    let mut no_baked = false;
    let mut max_bytes: Option<usize> = None;
    let mut chunks: usize = 1;
    let mut use_gpu = false;
    let mut use_hybrid = false;
    let mut second_mixer = false;
    let mut prior_cache: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--compare" => compare = true,
            "--no-baked" => no_baked = true,
            // The prior is compiled into every build that has this command, so
            // asking for it asks for the default, and `--no-baked` still wins.
            "--baked" => {}
            "--gpu" => use_gpu = true,
            "--hybrid" => use_hybrid = true,
            "--second-mixer" => second_mixer = true,
            "--prior-cache" => {
                i += 1;
                let Some(dir) = args.get(i) else {
                    eprintln!("trex compress: --prior-cache needs a directory");
                    return ExitCode::FAILURE;
                };
                prior_cache = Some(dir.clone());
            }
            "--text" => {
                i += 1;
                text = args.get(i).cloned();
            }
            "--max-bytes" => {
                i += 1;
                max_bytes = Some(flag_value_or_fail!("compress", args, i, "--max-bytes"));
            }
            "--chunks" => {
                // N independent chunks across cores (0 = all logical cores). Each
                // chunk seeds from the baked prior, so the ratio cost is bounded.
                i += 1;
                chunks = flag_value_or_fail!("compress", args, i, "--chunks");
            }
            "-h" | "--help" => {
                println!("trex compress (FILE | --text STRING) [--compare] [--no-baked]");
                println!("    report the achieved compression of the default coder");
                println!("    (logistic mix + orbit, auto-using the baked prior)");
                println!("    --compare    print the full coder comparison table");
                println!("    --no-baked   run corpus-free (skip the baked prior)");
                println!("    --prior-cache DIR   map the baked prior from DIR, writing it there on the");
                println!("                 first run, instead of decoding it every run");
                println!("    --second-mixer   a second mixer beside the shipped one, its weights chosen");
                println!("                 by the stream's rhythm (the power and the group delay of a");
                println!("                 resonator bank over the token kinds), the side information a");
                println!("                 decoder would need counted in; a code length 0.06-4.5% shorter");
                println!("                 on each of seven measured files, for up to 24% more time");
                return ExitCode::SUCCESS;
            }
            other if !other.starts_with('-') && file.is_none() => file = Some(other.to_string()),
            other => {
                eprintln!("trex compress: unknown argument: {other}");
                return ExitCode::FAILURE;
            }
        }
        i += 1;
    }
    let owned: Vec<u8> = if let Some(t) = &text {
        t.clone().into_bytes()
    } else if let Some(f) = &file {
        match read_input(f) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("trex compress: {f}: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        eprintln!("trex compress: no input (pass a FILE or --text STRING)");
        return ExitCode::FAILURE;
    };
    let bytes = match max_bytes {
        Some(k) => &owned[..k.min(owned.len())],
        None => &owned[..],
    };
    let n = bytes.len();
    if n == 0 {
        eprintln!("trex compress: empty input");
        return ExitCode::FAILURE;
    }
    if compare {
        print_compress_table("trex compress --compare", bytes, &[]);
        return ExitCode::SUCCESS;
    }
    // The second mixer is a CPU coder's; the device coders have none, and a run
    // that asked for it and coded without it would report the wrong coder.
    if second_mixer && (use_gpu || use_hybrid) {
        eprintln!("trex compress: --second-mixer runs on the CPU coder and cannot combine with --gpu or --hybrid");
        return ExitCode::FAILURE;
    }
    // Default coder: logistic mix + orbit over the baked prior; `--no-baked`
    // runs corpus-free.
    let use_baked = !no_baked;
    // The device coder's default prior is projected from the same baked blob,
    // so switching the baked prior off codes the device side with no prior.
    fn gpu_compress(bytes: &[u8], prior: bool) -> Option<f64> {
        if prior { trex::gpu::compress_gpu(bytes) } else { trex::gpu::compress_gpu_with_prior(bytes, &[0u16; 3], 0, false) }
    }
    let device_prior = use_baked;
    // The CPU coder reads the prior mapped from the cache directory when one is
    // named, and decoded otherwise; both code to the same bits.
    let mapped: Option<trex::prior_cache::MappedPrior> = match (&prior_cache, use_baked) {
        (Some(dir), true) => {
            let blob = seam::baked_model_blob();
            let t = std::time::Instant::now();
            match trex::prior_cache::MappedPrior::open_or_build(std::path::Path::new(dir), blob) {
                Ok((prior, built)) => {
                    let ms = t.elapsed().as_secs_f64() * 1e3;
                    if built {
                        eprintln!(
                            "trex compress: wrote the prior cache to {dir} ({} bytes) in {ms:.0} ms",
                            prior.bytes_on_disk()
                        );
                    } else {
                        eprintln!("trex compress: mapped the prior cache in {dir} in {ms:.2} ms");
                    }
                    Some(prior)
                }
                Err(e) => {
                    eprintln!("trex compress: cannot use the prior cache in {dir}: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
        (Some(dir), false) => {
            eprintln!("trex compress: --prior-cache {dir} has no prior to cache without the baked model");
            None
        }
        (None, _) => None,
    };
    let baked = if use_baked && mapped.is_none() { Some(seam::baked_model()) } else { None };
    let prior_name = match (&mapped, &baked) {
        (Some(_), _) => " + mapped prior",
        (None, Some(_)) => " + baked prior",
        (None, None) => " (corpus-free)",
    };
    let cpu_bits = |input: &[u8], n_chunks: usize| -> f64 {
        match (&mapped, n_chunks, second_mixer) {
            (Some(p), 1, false) => seam::logistic_mix_bits_with(input, true, &[], Some(p)),
            (Some(p), n, false) => seam::compress_chunks_with(input, n, Some(p)),
            (None, 1, false) => seam::logistic_mix_bits(input, true, &[], baked),
            (None, n, false) => seam::compress_chunks(input, n, baked),
            (Some(p), 1, true) => seam::second_mixer_bits_with(input, Some(p)),
            (Some(p), n, true) => seam::compress_chunks_second_mixer_with(input, n, Some(p)),
            (None, 1, true) => seam::second_mixer_bits_with(input, baked),
            (None, n, true) => seam::compress_chunks_second_mixer_with(input, n, baked),
        }
    };
    let second_name = if second_mixer { " + a second mixer by rhythm" } else { "" };
    let t0 = std::time::Instant::now();
    let (bits, coder) = if use_hybrid {
        // MIMT: the GPU codes the tail on its own thread while the CPU (Flynnel,
        // full model) codes the head concurrently; the GPU thread mostly waits
        // on the device, so the CPU cores stay busy. The CPU takes the smaller
        // share since the device is the faster of the two.
        let cpu_len = (n / 8).clamp(1, n);
        let gpu_part = bytes[cpu_len..].to_vec();
        let cpu_part = &bytes[..cpu_len];
        let gpu_handle = std::thread::spawn(move || gpu_compress(&gpu_part, device_prior));
        let head_bits = cpu_bits(cpu_part, 0);
        let device_bits = match gpu_handle.join() {
            Ok(b) => b,
            Err(panic) => {
                eprintln!("trex compress: the device thread panicked: {panic:?}");
                return ExitCode::FAILURE;
            }
        };
        match device_bits {
            Some(gb) => (
                head_bits + gb,
                format!("hybrid MIMT (CPU {cpu_len} B + GPU {} B, concurrent)", n - cpu_len),
            ),
            None => {
                eprintln!("trex compress: --hybrid but no device is available; CPU only");
                (head_bits + cpu_bits(&bytes[cpu_len..], 0), "CPU only (no device)".to_string())
            }
        }
    } else if use_gpu {
        match gpu_compress(bytes, device_prior) {
            Some(b) => (b, "GPU chunked coder (context + orbit + match)".to_string()),
            None => {
                eprintln!("trex compress: --gpu requested but no device is available; using the CPU coder");
                (cpu_bits(bytes, 1), format!("logistic mix + orbit{prior_name}"))
            }
        }
    } else if chunks == 1 {
        (cpu_bits(bytes, 1), format!("logistic mix + orbit{second_name}{prior_name}"))
    } else {
        (cpu_bits(bytes, chunks), format!("logistic mix + orbit{second_name}{prior_name}, {chunks} chunks"))
    };
    let secs = t0.elapsed().as_secs_f64();
    let out = (bits / 8.0).ceil() as u64;
    let bpb = bits / n as f64;
    let pct = 100.0 * out as f64 / n as f64;
    let mbps = (n as f64 / 1e6) / secs.max(1e-9);
    println!("trex compress: {n} bytes -> {out} bytes  ({bpb:.3} bits/byte, {pct:.1}% of original)");
    println!("  coder: {coder}   {mbps:.2} MB/s   (--compare for the full table)");
    ExitCode::SUCCESS
}

fn run_seam(args: &[String]) -> ExitCode {
    let mut input: Option<Vec<u8>> = None;
    let mut order: usize = 3;
    let mut show_field = false;
    let mut show_segment = false;
    let mut limit = usize::MAX;
    let mut recover = false;
    let mut compare_bpe = false;
    let mut json = false;
    let mut words: usize = 400;
    let mut passes_override: Option<usize> = None;
    let mut corpus_seed: u64 = 0x5eed_1234;
    let mut english = false;
    let mut grain = false;
    let mut max_bytes: Option<usize> = None;
    // The coder's options, which only a build with the `compress` feature has.
    #[cfg(feature = "compress")]
    let mut compress = false;
    #[cfg(feature = "compress")]
    let mut warm_file: Option<String> = None;
    #[cfg(feature = "compress")]
    let mut warm_bytes: Option<usize> = None;
    #[cfg(feature = "compress")]
    let mut build_model: Option<(String, String)> = None;
    #[cfg(feature = "compress")]
    let mut prune_model: Option<(String, String, f32)> = None;
    #[cfg(feature = "compress")]
    let mut model_vigilance: Option<f32> = None;
    #[cfg(feature = "compress")]
    let mut merge_model: Option<(String, String, f32, String)> = None;
    #[cfg(feature = "compress")]
    let mut model_topk: usize = 600_000;
    #[cfg(feature = "compress")]
    let mut bench_coder = false;
    #[cfg(feature = "compress")]
    let mut chunks: usize = 0; // 0 = all logical cores (the production default)
    #[cfg(feature = "compress")]
    let mut archive_dir: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--order" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --order needs a value");
                    return ExitCode::FAILURE;
                };
                let Ok(k) = v.parse::<usize>() else {
                    eprintln!("trex: --order needs a number");
                    return ExitCode::FAILURE;
                };
                order = k.max(1);
            }
            "--field" => show_field = true,
            "--segment" => show_segment = true,
            "--recover" => recover = true,
            "--compare-bpe" => compare_bpe = true,
            "--english" => english = true,
            "--limit" => {
                i += 1;
                limit = flag_value_or_fail!("seam", args, i, "--limit");
            }
            "--grain" => grain = true,
            #[cfg(not(feature = "compress"))]
            flag @ ("--compress" | "--warm-file" | "--warm-bytes" | "--build-model" | "--merge-model"
            | "--model-vigilance" | "--prune-model" | "--model-topk" | "--bench-coder"
            | "--archive-dir" | "--chunks") => {
                eprintln!("{}", compress_absent(&format!("trex seam {flag}")));
                return ExitCode::FAILURE;
            }
            #[cfg(feature = "compress")]
            "--compress" => compress = true,
            "--bytes" => {
                i += 1;
                max_bytes = Some(flag_value_or_fail!("seam", args, i, "--bytes"));
            }
            #[cfg(feature = "compress")]
            "--warm-file" => {
                i += 1;
                warm_file = args.get(i).cloned();
            }
            #[cfg(feature = "compress")]
            "--warm-bytes" => {
                i += 1;
                warm_bytes = Some(flag_value_or_fail!("seam", args, i, "--warm-bytes"));
            }
            #[cfg(feature = "compress")]
            "--build-model" => {
                let inp = args.get(i + 1).cloned();
                let outp = args.get(i + 2).cloned();
                i += 2;
                if let (Some(a), Some(b)) = (inp, outp) {
                    build_model = Some((a, b));
                }
            }
            #[cfg(feature = "compress")]
            "--merge-model" => {
                let a = args.get(i + 1).cloned();
                let b = args.get(i + 2).cloned();
                let w = args.get(i + 3).cloned();
                let o = args.get(i + 4).cloned();
                i += 4;
                let (Some(a), Some(b), Some(w), Some(o)) = (a, b, w, o) else {
                    eprintln!("trex seam: --merge-model needs A B WEIGHT OUT");
                    return ExitCode::FAILURE;
                };
                let w = match w.parse::<f32>() {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("trex seam: --merge-model weight {w:?} is not a number: {e}");
                        return ExitCode::FAILURE;
                    }
                };
                merge_model = Some((a, b, w, o));
            }
            #[cfg(feature = "compress")]
            "--model-vigilance" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex seam: --model-vigilance needs a value");
                    return ExitCode::FAILURE;
                };
                match v.parse::<f32>() {
                    Ok(r) => model_vigilance = Some(r),
                    Err(e) => {
                        eprintln!("trex seam: --model-vigilance {v:?} is not a number: {e}");
                        return ExitCode::FAILURE;
                    }
                }
            }
            #[cfg(feature = "compress")]
            "--prune-model" => {
                let inp = args.get(i + 1).cloned();
                let outp = args.get(i + 2).cloned();
                let rho = args.get(i + 3).cloned();
                i += 3;
                let (Some(a), Some(b), Some(r)) = (inp, outp, rho) else {
                    eprintln!("trex: --prune-model needs IN OUT RHO");
                    return ExitCode::FAILURE;
                };
                let r = match r.parse::<f32>() {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("trex: --prune-model vigilance {r:?} is not a number: {e}");
                        return ExitCode::FAILURE;
                    }
                };
                prune_model = Some((a, b, r));
            }
            #[cfg(feature = "compress")]
            "--model-topk" => {
                i += 1;
                model_topk = flag_value_or_fail!("seam", args, i, "--model-topk");
            }
            #[cfg(feature = "compress")]
            "--bench-coder" => bench_coder = true,
            #[cfg(feature = "compress")]
            "--archive-dir" => {
                i += 1;
                archive_dir = args.get(i).cloned();
            }
            #[cfg(feature = "compress")]
            "--chunks" => {
                i += 1;
                chunks = flag_value_or_fail!("seam", args, i, "--chunks");
            }
            "--json" => json = true,
            "--words" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --words needs a value");
                    return ExitCode::FAILURE;
                };
                let Ok(w) = v.parse::<usize>() else {
                    eprintln!("trex: --words needs a number");
                    return ExitCode::FAILURE;
                };
                words = w.max(1);
            }
            "--max-bytes" => {
                i += 1;
                let Some(v) = args.get(i).and_then(|s| s.parse::<usize>().ok()) else {
                    eprintln!("trex seam: --max-bytes needs a number");
                    return ExitCode::FAILURE;
                };
                max_bytes = Some(v);
            }
            "--seed" => {
                i += 1;
                let Some(v) = args.get(i).and_then(|v| v.parse::<u64>().ok()) else {
                    eprintln!("trex seam: --seed needs a number");
                    return ExitCode::FAILURE;
                };
                corpus_seed = v;
            }
            "--passes" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --passes needs a value");
                    return ExitCode::FAILURE;
                };
                let Ok(p) = v.parse::<usize>() else {
                    eprintln!("trex: --passes needs a number");
                    return ExitCode::FAILURE;
                };
                passes_override = Some(p);
            }
            "--text" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --text needs a value");
                    return ExitCode::FAILURE;
                };
                input = Some(v.clone().into_bytes());
            }
            path => match read_input(path) {
                Ok(b) => input = Some(b),
                Err(e) => {
                    eprintln!("trex: cannot read {path}: {e}");
                    return ExitCode::FAILURE;
                }
            },
        }
        i += 1;
    }

    use trex::seam::{self, SeamConfig};

    // Train the baked byte-level integer model from a corpus and write the
    // brotli blob (run once offline; the result is then `include_bytes!`d).
    #[cfg(feature = "compress")]
    if let Some((corpus_path, out_path)) = build_model {
        let corpus = match read_input(&corpus_path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("trex: cannot read {corpus_path}: {e}");
                return ExitCode::FAILURE;
            }
        };
        eprintln!("trex: training byte n-gram model on {} corpus bytes ...", corpus.len());
        let mut model = seam::byte_ngram_train(&corpus);
        // Vigilance before top-k, so the row budget is spent on rows that say
        // something their backoff parent does not. top-k ranks by total count,
        // which is frequency and not information.
        if let Some(rho) = model_vigilance {
            let (before, after) = seam::prune_counts(&mut model, rho);
            eprintln!("trex: vigilance {rho} kept {after} of {before} rows");
        }
        let blob = seam::serialize_byte_ngram(&model, model_topk);
        if let Err(e) = std::fs::write(&out_path, &blob) {
            eprintln!("trex: cannot write {out_path}: {e}");
            return ExitCode::FAILURE;
        }
        println!("trex: wrote {out_path} ({} bytes)", blob.len());
        return ExitCode::SUCCESS;
    }

    // Add one trained blob's counts into another's, scaling the second, and
    // write the result. Counts are observation tallies, so the sum is what
    // training on both corpora at once would have given.
    #[cfg(feature = "compress")]
    if let Some((a_path, b_path, weight, out_path)) = merge_model {
        let a = match std::fs::read(&a_path) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("trex: cannot read {a_path}: {e}");
                return ExitCode::FAILURE;
            }
        };
        let b = match std::fs::read(&b_path) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("trex: cannot read {b_path}: {e}");
                return ExitCode::FAILURE;
            }
        };
        let (blob, rows_a, rows_b, rows_out, saturated) = seam::merge_baked(&a, &b, weight);
        if let Err(e) = std::fs::write(&out_path, &blob) {
            eprintln!("trex: cannot write {out_path}: {e}");
            return ExitCode::FAILURE;
        }
        println!(
            "trex: wrote {out_path} ({} bytes), rows {rows_a} + {rows_b} -> {rows_out} \
             at weight {weight}",
            blob.len()
        );
        if saturated > 0 {
            println!(
                "trex: {saturated} symbol counts reached the format's ceiling and were clamped"
            );
        }
        return ExitCode::SUCCESS;
    }

    // Drop rows of a trained blob that predict what their backoff parent already
    // predicts, and write the thinner blob (run once offline, like --build-model;
    // the result is then `include_bytes!`d).
    //
    // A vigilance above one cannot be reached by an overlap, so it drops nothing
    // and round-trips the input: that is how a run of this can be checked against
    // the blob it started from.
    #[cfg(feature = "compress")]
    if let Some((in_path, out_path, rho)) = prune_model {
        let blob = match std::fs::read(&in_path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("trex: cannot read {in_path}: {e}");
                return ExitCode::FAILURE;
            }
        };
        let (out, before, after) = seam::prune_baked(&blob, rho);
        if let Err(e) = std::fs::write(&out_path, &out) {
            eprintln!("trex: cannot write {out_path}: {e}");
            return ExitCode::FAILURE;
        }
        println!(
            "trex: wrote {out_path} ({} bytes from {}), rows {before} -> {after} at vigilance {rho}",
            out.len(),
            blob.len()
        );
        return ExitCode::SUCCESS;
    }

    // Hierarchical grain separation on a synthetic instruction-class stream:
    // multi-scale persistence and prologue-orbit recognition versus the flat
    // single-threshold pass that cannot separate function grain from block
    // grain. `--words` scales the function count.
    if grain {
        let p = seam::synth_program(words.max(8), 0x6a11_5eed);
        let r_flat = seam::recovery(
            &seam::boundaries_at_tier(&seam::persistence(&p.classes, &[4.0], 0.3), 1),
            &p.func_starts,
            1,
        );
        let taus = [4.0, 12.0, 32.0];
        let pers = seam::persistence(&p.classes, &taus, 1.0);
        let r_func =
            seam::recovery(&seam::boundaries_at_tier(&pers, taus.len() as u8), &p.func_starts, 1);
        let r_block = seam::recovery(&seam::boundaries_at_tier(&pers, 1), &p.block_starts, 1);
        let r_pro =
            seam::recovery(&seam::prologue_starts(&p.classes, &seam::PROLOGUE), &p.func_starts, 0);
        println!(
            "trex seam --grain: {} instructions, {} functions, {} blocks (synthetic)",
            p.classes.len(),
            p.func_starts.len(),
            p.block_starts.len()
        );
        println!(
            "  flat single-threshold  (function): P={:.3} R={:.3} F1={:.3}  <- the conflation",
            r_flat.precision, r_flat.recall, r_flat.f1
        );
        println!(
            "  multi-scale coarse tier(function): P={:.3} R={:.3} F1={:.3}",
            r_func.precision, r_func.recall, r_func.f1
        );
        println!(
            "  multi-scale fine tier  (block)   : P={:.3} R={:.3} F1={:.3}",
            r_block.precision, r_block.recall, r_block.f1
        );
        println!(
            "  prologue-orbit         (function): P={:.3} R={:.3} F1={:.3}",
            r_pro.precision, r_pro.recall, r_pro.f1
        );
        // Unified: texture discovers the marker, the marker completes the
        // coarse grain, fine texture fills the inner grain - both axes, no
        // marker known in advance.
        let ug = seam::unified_grain(&p.classes, &taus, 1.0, seam::PROLOGUE.len());
        let r_uf = seam::recovery(&ug.func_starts, &p.func_starts, 1);
        let r_ub = seam::recovery(&ug.block_starts, &p.block_starts, 1);
        println!(
            "  UNIFIED  discovered coarse marker {:?} (the recurring inter-function transition):",
            ug.marker
        );
        println!(
            "    function (texture discovers, marker completes, +/-1): P={:.3} R={:.3} F1={:.3}",
            r_uf.precision, r_uf.recall, r_uf.f1
        );
        println!(
            "    block    (function anchors + texture)  : P={:.3} R={:.3} F1={:.3}",
            r_ub.precision, r_ub.recall, r_ub.f1
        );
        return ExitCode::SUCCESS;
    }

    // Cross-file dedup probe: compress every file in a directory two ways - each
    // file independently (no cross-file sharing) versus the concatenation coded
    // as one stream, where the match model spans file boundaries and dedupes
    // shared content. The "ladder" to split the stream back into files is just
    // the per-file lengths (8 bytes each, counted below). On a corpus with
    // cross-file redundancy the concatenated cost is far lower; on unique files
    // the two tie - the empirical test of the slice-and-connect idea.
    #[cfg(feature = "compress")]
    if let Some(dir) = &archive_dir {
        let baked = seam::baked_model();
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        match std::fs::read_dir(dir) {
            Ok(rd) => {
                for entry in rd.flatten() {
                    let p = entry.path();
                    if p.is_file()
                        && let Ok(b) = read_input(&p)
                        && !b.is_empty()
                    {
                        files.push((p.display().to_string(), b));
                    }
                }
            }
            Err(e) => {
                eprintln!("trex seam --archive-dir: cannot read {dir}: {e}");
                return ExitCode::FAILURE;
            }
        }
        if files.is_empty() {
            eprintln!("trex seam --archive-dir: no readable files in {dir}");
            return ExitCode::FAILURE;
        }
        files.sort_by(|a, b| a.0.cmp(&b.0));
        let mut indep_bits = 0.0f64;
        let mut concat: Vec<u8> = Vec::new();
        let mut total = 0usize;
        for (_, b) in &files {
            indep_bits += seam::logistic_mix_bits(b, true, &[], Some(baked));
            concat.extend_from_slice(b);
            total += b.len();
        }
        let concat_bits = seam::logistic_mix_bits(&concat, true, &[], Some(baked));
        let ladder_bits = files.len() as f64 * 64.0; // one u64 length per boundary
        let tf = total as f64;
        println!("archive: {} files, {total} bytes", files.len());
        println!("  independent per-file: {:.4} bpb  ({indep_bits:.0} bits)", indep_bits / tf);
        println!(
            "  concatenated stream : {:.4} bpb  ({concat_bits:.0} bits + {ladder_bits:.0} ladder)  [match model dedupes cross-file]",
            (concat_bits + ladder_bits) / tf
        );
        let saving = (1.0 - (concat_bits + ladder_bits) / indep_bits) * 100.0;
        println!("  cross-file dedup saving: {saving:.1}%");
        return ExitCode::SUCCESS;
    }

    // Predictive compression: corpus-free, adaptive, multi-scale (and
    // orbit-context) bits/byte versus BPE tokens plus codebook.
    #[cfg(feature = "compress")]
    if compress {
        let Some(full) = input.as_deref() else {
            eprintln!("trex seam --compress: no input (pass a FILE or --text STRING)");
            return ExitCode::FAILURE;
        };
        let bytes = match max_bytes {
            Some(k) => &full[..k.min(full.len())],
            None => full,
        };
        let n = bytes.len();
        if n == 0 {
            eprintln!("trex seam --compress: empty input");
            return ExitCode::FAILURE;
        }
        let nf = n as f64;
        if bench_coder {
            // Time only the shipped coder. With --chunks N
            // it slices the input across N cores (each from the baked prior).
            let t0 = std::time::Instant::now();
            let bits = if chunks == 1 {
                seam::logistic_mix_bits(bytes, true, &[], Some(seam::baked_model()))
            } else {
                seam::compress_chunks(bytes, chunks, Some(seam::baked_model()))
            };
            let dt = t0.elapsed().as_secs_f64();
            println!(
                "bench-coder: {n} bytes  {chunks} chunks  {:.4} bpb  {dt:.3} s  {:.1} ns/byte",
                bits / nf,
                dt * 1e9 / nf
            );
            return ExitCode::SUCCESS;
        }
        let warm_seed: Vec<u8> = match &warm_file {
            Some(p) => {
                let mut s = read_input(p).unwrap_or_default();
                if let Some(k) = warm_bytes {
                    s.truncate(k);
                }
                s
            }
            None => Vec::new(),
        };
        print_compress_table("trex seam --compress", bytes, &warm_seed);
        return ExitCode::SUCCESS;
    }

    // The boundary-recovery benchmark: segment spaceless text with no
    // dictionary and score against the known word boundaries, optionally
    // against the count-BPE baseline.
    if recover {
        const VOCAB: &[&str] = &[
            "data", "stream", "model", "token", "signal", "past", "future", "read", "byte",
            "scale", "field", "probe", "cortex", "lattice", "vector",
        ];
        // With a file, the corpus is that file's real words despaced - the honest test, because
        // the synthetic vocabulary below is 15 words that recur constantly and flatters any
        // recurrence rule. Without one, the synthetic corpus stands in, and its seed is a flag
        // rather than a constant so a comparison can be run over a spread rather than one draw.
        let (bytes, truth) = match &input {
            Some(text) => seam::despaced_text(text, max_bytes.unwrap_or(1 << 17)),
            None => seam::spaceless_corpus(VOCAB, words, corpus_seed),
        };
        let cfg = SeamConfig { order, passes: passes_override.unwrap_or(0), ..SeamConfig::default() };
        let used_passes = if cfg.passes == 0 { seam::auto_passes(bytes.len()) } else { cfg.passes };
        let field = seam::analyze_with(&bytes, &cfg);
        let rs = seam::recovery(&field.internal_cuts(), &truth, 1);
        // `--words` sizes the synthetic corpus only.
        let corpus = if input.is_some() { String::new() } else { format!(" ({words} words)") };
        println!(
            "trex seam --recover: {} bytes{corpus}, {} true boundaries, order {order}, passes {used_passes}",
            bytes.len(),
            truth.len()
        );
        println!(
            "  seam (no dictionary):   P={:.3} R={:.3} F1={:.3}  ({} cuts, {} hit)",
            rs.precision, rs.recall, rs.f1, rs.predicted, rs.hit
        );
        if compare_bpe {
            let bcuts = seam::bpe_boundaries(&bytes, 200);
            let rb = seam::recovery(&bcuts, &truth, 1);
            println!(
                "  count-BPE (200 merges): P={:.3} R={:.3} F1={:.3}  ({} piece boundaries)",
                rb.precision, rb.recall, rb.f1, rb.predicted
            );
            println!(
                "  -> seam recovers word boundaries {:+.3} F1 over count-BPE: the bidirectional",
                rs.f1 - rb.f1
            );
            println!("     predictive signal (past<->future branching entropy) BPE has no access to.");
        }
        return ExitCode::SUCCESS;
    }

    let Some(input) = input else {
        eprintln!("trex seam: no input given (pass a FILE, --text STRING, or --recover)");
        return ExitCode::FAILURE;
    };
    let cfg = SeamConfig { order, passes: passes_override.unwrap_or(0), ..SeamConfig::default() };
    let field = if english {
        seam::analyze_with_model(&input, &seam::english_model(), &cfg)
    } else {
        seam::analyze_with(&input, &cfg)
    };

    if json {
        print!("{{\"len\":{},\"cuts\":[", field.len);
        for (k, c) in field.cuts.iter().enumerate() {
            if k > 0 {
                print!(",");
            }
            print!("{c}");
        }
        print!("],\"fwd\":[");
        for (k, v) in field.fwd_entropy.iter().enumerate() {
            if k > 0 {
                print!(",");
            }
            print!("{v:.3}");
        }
        print!("],\"bwd\":[");
        for (k, v) in field.bwd_entropy.iter().enumerate() {
            if k > 0 {
                print!(",");
            }
            print!("{v:.3}");
        }
        println!("]}}");
        return ExitCode::SUCCESS;
    }

    let segs = field.segments();
    println!(
        "trex seam: {} bytes, order {order}, {} segments (bidirectional branching entropy)",
        field.len,
        segs.len()
    );

    if show_field {
        println!("  per-byte branching entropy:");
        for (t, &b) in input.iter().enumerate().take(limit) {
            let c = if b.is_ascii_graphic() { b as char } else { '.' };
            println!("    {t:>6} {c}  fwd={:.2} bwd={:.2}", field.fwd_entropy[t], field.bwd_entropy[t]);
        }
        if input.len() > limit {
            println!("    ... (+{} more bytes; raise --limit)", input.len() - limit);
        }
    }

    if show_segment || !show_field {
        println!("  segments:");
        for (s, e) in segs.iter().take(limit) {
            let shown: String = String::from_utf8_lossy(&input[*s..*e]).chars().take(40).collect();
            println!("    [{s:>6}..{e:<6}] {shown:?}");
        }
        if segs.len() > limit {
            println!("    ... (+{} more segments; raise --limit)", segs.len() - limit);
        }
    }

    ExitCode::SUCCESS
}

fn run_orbit(args: &[String]) -> ExitCode {
    use trex::orbit::{self, OrbitGroup};
    let mut input: Option<Vec<u8>> = None;
    let mut group = OrbitGroup::Shape;
    let mut collapse = false;
    let mut boundary = false;
    let mut query: Option<Vec<u8>> = None;
    let mut limit = usize::MAX;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--group" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --group needs a value");
                    return ExitCode::FAILURE;
                };
                let Some(g) = OrbitGroup::parse(v) else {
                    eprintln!("trex: unknown group {v:?} (identity|case|notation|shape|e8|ip|url|time|path|fold|numeric, or a typed relation such as subnet/24)");
                    return ExitCode::FAILURE;
                };
                group = g;
            }
            "--collapse" => collapse = true,
            "--boundary" => boundary = true,
            "--limit" => {
                i += 1;
                limit = flag_value_or_fail!("orbit", args, i, "--limit");
            }
            "--match" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --match needs a query");
                    return ExitCode::FAILURE;
                };
                query = Some(v.clone().into_bytes());
            }
            "--text" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --text needs a value");
                    return ExitCode::FAILURE;
                };
                input = Some(v.clone().into_bytes());
            }
            path => match read_input(path) {
                Ok(b) => input = Some(b),
                Err(e) => {
                    eprintln!("trex: cannot read {path}: {e}");
                    return ExitCode::FAILURE;
                }
            },
        }
        i += 1;
    }
    let Some(input) = input else {
        eprintln!("trex orbit: no input given (pass a FILE or --text STRING)");
        return ExitCode::FAILURE;
    };

    // Equivariant match: every span in the query's orbit (matches up to the group).
    if let Some(q) = query {
        let hits = orbit::matches(&input, &q, group);
        println!(
            "trex orbit --match {:?} (group {}): {} spans in the same orbit",
            String::from_utf8_lossy(&q),
            group.label(),
            hits.len()
        );
        for t in hits.iter().take(limit) {
            println!("    [{:>6}..{:<6}] {:?}", t.start, t.end, t.raw);
        }
        return ExitCode::SUCCESS;
    }

    // Orbit-change boundary: cut where the symbol kind changes.
    if boundary {
        let cuts = orbit::shape_boundaries(&input);
        println!("trex orbit --boundary: {} symbol-kind transitions", cuts.len());
        let mut prev = 0usize;
        let mut segs: Vec<(usize, usize)> = cuts.iter().map(|&c| (std::mem::replace(&mut prev, c), c)).collect();
        segs.push((prev, input.len()));
        for (s, e) in segs.iter().take(limit) {
            println!("    [{s:>6}..{e:<6}] {:?}", String::from_utf8_lossy(&input[*s..*e]));
        }
        return ExitCode::SUCCESS;
    }

    // Orbit-collapse: distinct raw forms folding onto each orbit representative.
    if collapse {
        let table = orbit::collapse(&input, group);
        let (raw, orb) = orbit::collapse_stats(&input, group);
        println!(
            "trex orbit --collapse (group {}): {raw} raw forms -> {orb} orbits ({}x reduction)",
            group.label(),
            if orb > 0 { raw as f32 / orb as f32 } else { 1.0 }
        );
        for (o, forms) in table.iter().take(limit) {
            if forms.len() > 1 {
                println!("    {o:?}  <-  {forms:?}");
            } else {
                println!("    {o:?}");
            }
        }
        return ExitCode::SUCCESS;
    }

    // Default: list each token with its orbit representative.
    let toks = orbit::tokenize(&input, group);
    println!("trex orbit (group {}): {} tokens", group.label(), toks.len());
    for t in toks.iter().take(limit) {
        println!("    [{:>6}..{:<6}] {:?} -> {:?}", t.start, t.end, t.raw, t.orbit);
    }
    ExitCode::SUCCESS
}

fn run_prefilter(args: &[String]) -> ExitCode {
    use trex::prefilter::{BloomFilter, CuckooFilter, Membership, XorFilter};

    let mut corpus: Option<Vec<u8>> = None;
    let mut literals: Vec<Vec<u8>> = Vec::new();
    let mut which = String::from("bloom");
    let mut verify = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--text" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --text needs a value");
                    return ExitCode::FAILURE;
                };
                corpus = Some(v.clone().into_bytes());
            }
            "--literal" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --literal needs a value");
                    return ExitCode::FAILURE;
                };
                literals.push(v.clone().into_bytes());
            }
            "--filter" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --filter needs a value");
                    return ExitCode::FAILURE;
                };
                which = v.clone();
            }
            "--verify" => verify = true,
            path => match read_input(path) {
                Ok(b) => corpus = Some(b),
                Err(e) => {
                    eprintln!("trex: cannot read {path}: {e}");
                    return ExitCode::FAILURE;
                }
            },
        }
        i += 1;
    }

    let Some(corpus) = corpus else {
        eprintln!("trex prefilter: no corpus given (pass a FILE or --text STRING)");
        return ExitCode::FAILURE;
    };

    let filter: Box<dyn Membership> = match which.as_str() {
        "bloom" => Box::new(BloomFilter::build(&corpus)),
        "cuckoo" => Box::new(CuckooFilter::build(&corpus)),
        "xor" => Box::new(XorFilter::build(&corpus)),
        other => {
            eprintln!("trex: unknown filter {other:?} (use bloom, cuckoo, or xor)");
            return ExitCode::FAILURE;
        }
    };

    for lit in &literals {
        let shown = String::from_utf8_lossy(lit);
        if filter.might_contain(lit) {
            // An approximate yes; confirm with an exact search so the
            // report separates a true hit from a filter false positive.
            let exact = trex::byte_simd::contains(&corpus, lit);
            let verdict = if exact { "present (confirmed)" } else { "filter false positive" };
            println!("{}: {shown:?} -> might occur; {verdict}", filter.name());
        } else {
            println!("{}: {shown:?} -> ABSENT (rejected with no corpus scan)", filter.name());
        }
    }

    if verify {
        return run_prefilter_verify(&corpus);
    }
    ExitCode::SUCCESS
}

/// Verify the no-false-negative contract over a corpus: every literal
/// that truly occurs must be reported as a possible match by all three
/// filters. Also report how many genuinely-absent probes each filter
/// rejects, which is the scan-saving the prefilter buys.
fn run_prefilter_verify(corpus: &[u8]) -> ExitCode {
    let checks = trex::prefilter::verify(corpus);
    println!("verification over a {} byte corpus", corpus.len());
    if let Some(first) = checks.first() {
        println!("  present probes: {}   absent probes: {}", first.present, first.absent);
    }
    let mut all_ok = true;
    for c in &checks {
        let reject_pct = 100.0 * c.rejected as f64 / c.absent as f64;
        println!(
            "  {:>6}: {} false negatives, {}/{} absent rejected ({reject_pct:.1}% scans saved)",
            c.name, c.false_negatives, c.rejected, c.absent
        );
        if c.false_negatives != 0 {
            all_ok = false;
        }
    }
    if all_ok {
        println!("zero false negatives across all filters: the contract holds.");
        ExitCode::SUCCESS
    } else {
        eprintln!("FALSE NEGATIVE DETECTED: a present literal was reported absent.");
        ExitCode::FAILURE
    }
}
