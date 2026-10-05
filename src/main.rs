//! The `trex` command-line entry point.
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

/// Whether `arg` reads as a flag: a dash with something after it. A lone `-`
/// names the standard input.
pub(crate) fn is_flag(arg: &str) -> bool {
    arg.len() > 1 && arg.starts_with('-')
}

/// Refuse `flag`, which `command` does not take, naming where its flags are
/// listed and how a path that begins with a dash is given.
pub(crate) fn unknown_flag(command: &str, flag: &str) -> ExitCode {
    eprintln!("trex {command}: unknown flag {flag}");
    eprintln!("run `trex {command} --help` for its flags; a path that begins with a dash goes after --");
    ExitCode::FAILURE
}

/// Whether the arguments open with `-h` or `--help`: the check a command
/// makes before it reads the positional arguments it needs first.
pub(crate) fn opens_with_help(args: &[String]) -> bool {
    matches!(args.first().map(String::as_str), Some("-h" | "--help"))
}

mod cli_files;
mod cli_follow;
mod cli_lines;
mod cli_rules;
mod cli_tokens;
mod cli_window;
mod out;

/// Where Flynnel's notes go: nowhere, since the console carries only what a
/// command prints and TREX keeps no log to write them to.
fn flynnel_note(_note: &str) {}

fn main() -> ExitCode {
    // The sink is installed before any command runs, so it receives the
    // notes of the first parallel run, a calibration Flynnel measured among
    // them.
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
        Some("tokens") => cli_tokens::run_tokens(&args[1..]),
        Some("escape") => cli_tokens::run_escape(&args[1..]),
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
        Some("gravity") => run_gravity(&args[1..]),
        Some("context") => run_context(&args[1..]),
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
/// arguments, wherever they are, and set how every scan in this process
/// reads a timestamp. `--now` is a timestamp with a date in any form the
/// lexer recognizes or seconds since the epoch; `--tz` is the offset given to
/// a timestamp written with no zone; `--date-order` says which of the two
/// leading fields of an all-numeric slash date is the day where both
/// readings are valid dates.
fn take_clock_args(args: &mut Vec<String>) -> Result<(), String> {
    let mut i = 0;
    while i < args.len() {
        // What follows `--` is a path or a value, whatever it spells.
        if args[i] == "--" {
            break;
        }
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

/// Reads the declaration at `args[$i]`, `--lib` or one of the inline flags a
/// scan takes, into `$decls` and moves on to the next argument; the command
/// fails on a refused one, and any other argument falls through.
macro_rules! take_declaration_or_fail {
    ($args:expr, $i:ident, $decls:ident) => {
        let read = match $args[$i].as_str() {
            "--lib" => {
                $i += 1;
                Some(match $args.get($i) {
                    Some(path) => cli_files::declare_file(&mut $decls, path).map_err(|e| e.to_string()),
                    None => Err("--lib needs a pattern file, or a directory of .trex files".to_string()),
                })
            }
            flag @ ("--shape" | "--shape-after" | "--kind" | "--let" | "--declare") => {
                $i += 1;
                Some(cli_files::declare_flag(&mut $decls, flag, $args.get($i)))
            }
            _ => None,
        };
        match read {
            Some(Ok(())) => {
                $i += 1;
                continue;
            }
            Some(Err(e)) => {
                eprintln!("trex: {e}");
                return ExitCode::FAILURE;
            }
            None => {}
        }
    };
}

/// The tokens of `input` as the declarations `decls` lex them: every token,
/// or only the significant ones, which leaves out whitespace.
fn lex_declared(input: &[u8], decls: &trex::Declarations, significant: bool) -> Vec<trex::token::Token> {
    let shapes = decls.set();
    let toks = if shapes.is_empty() {
        trex::lexer::lex(input)
    } else {
        trex::lexer::lex_with_shapes(input, &trex::lexer::blob_runs(input), shapes, 0)
    };
    if significant { toks.into_iter().filter(|t| t.is_significant()).collect() } else { toks }
}

/// [`flag_value`] read as a finite `f32` cut, or the command failing: a cut
/// of `NaN` would pass no point and an infinite one every point or none.
macro_rules! cut_value_or_fail {
    ($command:expr, $args:expr, $i:expr, $flag:expr) => {{
        let v: f32 = flag_value_or_fail!($command, $args, $i, $flag);
        if !v.is_finite() {
            eprintln!("trex {}: {} takes a finite number, not {v}", $command, $flag);
            return ExitCode::FAILURE;
        }
        v
    }};
}

/// Print what `trex COMMAND --help` shows for an axis command: its usage,
/// what it reads, and each flag with the default a setting takes, then the
/// declarations a command that reads tokens takes.
fn print_axis_help(command: &str) {
    let text = match command {
        "magnitude" => "\
usage: trex magnitude (FILE | --text STRING) [--field] [--jumps] [--outliers] [--energy] [--top K]
                      [--jump-threshold K] [--outlier-sigma S] [--limit N] [DECLARATIONS]
  each token's order of magnitude, its change from the token before, and its local energy
  --field                every token's reading
  --jumps                the scale discontinuities: a change of at least K orders of magnitude from
                         the token before, K being --jump-threshold (3 by default)
  --outliers             the tokens at least S standard deviations from the mean magnitude, S being
                         --outlier-sigma (2.5 by default)
  --energy               the heaviest points by local energy, the sum of squared magnitudes over
                         the 16 tokens before; --top K of them (3 by default)
",
        "stress" => "\
usage: trex stress (FILE | --text STRING) [--field] [--peaks] [--fractures] [--peak-min-depth D]
                   [--fracture-min-depth D] [--limit N] [DECLARATIONS]
  each token's bracket depth, the strain of the brackets held open around it, and their load
  --field                every token's reading
  --peaks                the stress peaks: local maxima of depth at least --peak-min-depth deep
                         (2 by default)
  --fractures            where a run of closing brackets releases from a level at least
                         --fracture-min-depth deep (2 by default)
",
        "flow" => "\
usage: trex flow (FILE | --text STRING) [--signal magnitude|stress|length] [--grain token|super]
                 [--field] [--reversals] [--analytic] [--window N] [--steady-band B] [--limit N]
                 [DECLARATIONS]
  the trend of a signal along the input: its slope, direction, momentum and reversals
  --signal S             the signal followed: magnitude (the default), stress, or token length
  --grain G              token (the default) or super, a reading per supertoken
  --window N             the trailing units the slope is averaged over (4 by default)
  --steady-band B        a slope within B of zero reads as steady (0.05 by default)
  --field                every unit's slope, direction and momentum
  --reversals            where the direction turns
  --analytic             every unit's amplitude, phase and frequency, the signal's analytic reading
",
        "context" => "\
usage: trex context (FILE | --text STRING) [--fold F] [--field] [--period] [--agreement]
                    [--token-window N] [--unit-window N] [--limit N] [DECLARATIONS]
  what each token is read against: the window before it, the supertokens around it, the
  brackets enclosing it, its key's earlier occurrences and values, the stretch since the
  texture last changed, and its column of the record period
  --fold F               the context read: window (the default), unit, units, enclosing, echo,
                         regime, phase or key, as \\N{>+1:F} names it
  --field                every token's context: how many tokens it folds and each axis's
                         reading over them
  --period               every token's column of the record period
  --agreement            every supertoken's role and how far from its start, in tokens, the
                         nearest spectral change point, shape change point and seam cut are
  --token-window N       the significant tokens a window folds (32 by default); the window
                         before a token is the one ending at the token before it
  --unit-window N        the supertokens the unit window folds (8 by default)
",
        "gravity" => "\
usage: trex gravity (FILE | --text STRING) [--grain byte|token|super] [--field] [--strain] [--bound]
                    [--classes] [--top K] [--limit N] [DECLARATIONS]
  the pair field: how each unit's past pushes it away, how strongly the input holds together
  across the cut before it, and which types the field places together
  --grain G              token (the default), byte, or super, a reading per supertoken
  --field                every unit's strain in bits and bound in bits per pair, each with its
                         percentile among the input's readings, and its gravity class; the first
                         unit reads neither
  --strain               the units under the most strain, --top K of them (8 by default)
  --bound                the cuts held together least, --top K of them
  --classes              each gravity class with its types and how many units are of them
  declarations are read at the token and super grains; the byte grain reads no tokens
",
        "observe" => "\
usage: trex observe (FILE | --text STRING) [--grain byte|token|super] [--field] [--contested]
                    [--contested-threshold T] [--contested-min-gap N] [DECLARATIONS]
  where the reading of what comes before a point and of what comes after it disagree
  --grain G              byte (the default), token, or super, a reading per supertoken
  --contested-threshold T  a contested point's disagreement reaches T, from 0 to 1 (0.2 by default)
  --contested-min-gap N  at the byte grain, of two contested points closer than N bytes the stronger
                         is kept (8 by default)
  --field                every unit's three readings and their disagreement
  --contested            the contested points
  declarations are read at the token and super grains; the byte grain reads no tokens
",
        "spectral" => "\
usage: trex spectral (FILE | --text STRING) [--segment] [--bands] [--timeline] [--classify]
                     [--cp-threshold K] [--cp-floor F] [--cp-min-gap N] [--json] [--limit N]
  the bytes as a signal: entropy, period, novelty and texture, and where they change
  --segment              the change points: where the reading moves more than K spreads past its
                         mean, K being --cp-threshold (4 by default), and where it then comes
                         back to the texture it left
  --cp-floor F           the threshold stays at least F of the mean above it (0.05 by default)
  --cp-min-gap N         two change points are at least N bytes apart (4 by default)
  --bands                one row a frame: the class mix, entropy, period, novelty and texture
  --timeline             the texture timeline beside --segment or --bands; shown by default otherwise
  --classify             the regions read by texture and token shape together: table, code, prose,
                         blob, numeric or mixed
  --json                 the frames and change points as JSON
",
        "shape" => "\
usage: trex shape (FILE | --text STRING) [--classes] [--period] [--segment] [--group G]
                  [--template-strength S] [--json] [--limit N] [DECLARATIONS]
  each token's silhouette, the period the silhouettes repeat at, and the template regions
  --classes              every token's shape class, period, period strength and novelty
  --period               the dominant shape period and the template regions: runs whose period
                         strength reaches --template-strength (0.6 by default)
  --segment              the shape change points
  --group G              read the silhouettes under an orbit group: identity, case, notation,
                         shape, e8, ip, url, time, path, fold, numeric, or a typed relation such
                         as subnet/24, domain or day
  --json                 the field as JSON: the dominant period, the change points, the template
                         regions and every token's frame
",
        "echo" => "\
usage: trex echo (FILE | --text STRING) [--field] [--group G] [--structure] [--top K]
                 [--max-period-cv C] [--limit N] [DECLARATIONS]
  how often each token's content recurs, at what spacing, and what is new
  --group G              the key read up to a symmetry or a representation: identity (the default),
                         case, shape, notation, e8, ip, url, time, path, fold, numeric, or a typed
                         relation such as subnet/24, domain or day
  --structure            the recurring supertoken structures, keyed by role and token-kind silhouette
  --top K                the size of each ranked list (8 by default)
  --max-period-cv C      a recurrence is periodic when its spacings vary by at most C of their
                         mean (0.3 by default)
  --field                every token's count, lag and period
",
        "relation" => "\
usage: trex relation (FILE | --text STRING) [--edges] [--field] [--canonical] [--limit N]
                     [DECLARATIONS]
  the graph the brackets and names make: enclosure, operator and adjacency edges, the reuse chords
  between scopes, and the readings of that graph
  --edges                every edge, then every reuse chord with its residual
  --field                every enclosed token's depth and enclosing heads
  --canonical            the alpha-equivalence form: binders as #, bound uses as ^k
",
        "orbit" => "\
usage: trex orbit (FILE | --text STRING) [--group G] [--collapse] [--boundary] [--same-as TEXT]
                  [--limit N] [DECLARATIONS]
  each token under a symmetry group, and what the group folds together
  --group G              identity, case, notation, shape (the default), e8, ip, url, time, path,
                         fold, numeric, or a typed relation such as subnet/24, domain or day
  --collapse             the orbits and the forms each folds, with the vocabulary reduction
  --boundary             the spans of one kind of character
  --same-as TEXT         every span in the orbit of TEXT
",
        "seam" => "\
usage: trex seam (FILE | --text STRING) [--order K] [--passes N] [--english] [--cut-threshold K]
                 [--field] [--segment] [--json] [--limit N]
       trex seam --recover [FILE] [--compare-bpe] [--words N] [--seed N] [--max-bytes N]
       trex seam --grain-separation [--words N] [--seed N]
  bidirectional predictive segmentation: a cut where the past stops predicting the future
  --order K              the longest context, in bytes (3 by default)
  --passes N             the confidence passes (1 by default)
  --english              score the seams against the built-in English letter model
  --cut-threshold K      a cut's strength is at least K standard deviations above the mean
                         strength (0.6 by default)
  --field                the forward and backward branching entropy at every byte, and the
                         strength of a cut before it
  --segment              the segments, which are printed by default
  --json                 the cuts, both entropies and the strengths as JSON
  --recover              word-boundary recovery on despaced text, against a count BPE with
                         --compare-bpe; a synthetic corpus where no FILE is named
  --grain-separation     function and block grain on a synthetic instruction stream
",
        _ => "",
    };
    print!("{text}");
    if command != "observe" {
        println!("  --limit N              at most N rows of a listing");
    }
    if !matches!(command, "spectral" | "seam") {
        println!("  DECLARATIONS           --lib FILE|DIR, --shape DECL, --shape-after DECL, --kind DECL,");
        println!("                         --let DECL and --declare LINE, read in order: the input is");
        println!("                         lexed as a scan with them would lex it");
    }
    println!("  a FILE that begins with a dash goes after --");
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
    println!("      --kind 'name = PATTERN'         declare a kind: each match of PATTERN, after the");
    println!("                                      lex, is one token \\{{name}} matches");
    println!("      --let 'name = PATTERN'          declare a sub-pattern \\{{name}} reads in place");
    println!("      --declare LINE                  declare any line a pattern file holds");
    println!("      --now TIMESTAMP                 the instant a \\T{{age<24h}} or \\T{{<now}} clause reads");
    println!("      --tz OFFSET                     the zone a timestamp written with none is read in");
    println!("      --explain                       under each match: the kinds it spans, the guard each");
    println!("                                      passed, every axis value read there, the route that answered");
    println!("      --format TEMPLATE               one line per match from a template: ${{0}}, ${{name:acc}},");
    println!("                                      ${{path}}, ${{line}}, ${{col}}, ${{start}}, ${{end}}; \\t and \\n");
    println!("      --head N --tail N --lines A..B  scan only those records of each input, lines unless");
    println!("                                      --record names another unit; the input's own line numbers");
    println!("      --follow                        then scan what each file gains as it grows; a file");
    println!("                                      truncated or replaced starts its -m count again");
    println!("      --keep-count                    unless this keeps counting it");
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
    println!("      --lib FILE|DIR                  declarations for \\{{name}}: let, kind, shape, test lines;");
    println!("                                      a directory reads as every .trex file under it");
    println!("      --shape --kind --let --declare  the declarations scan takes, read in order with --lib");
    println!("      -m N                            the first N matches of each input alone");
    println!("      -i [--explain] [--show-skipped] put each change to you first; -U applies every one");
    println!("    trex redact PATTERN [INPUT]...    mask every match, one * per character");
    println!("      --keep 'card:last4, ip:octet1-2' leave the named fields unmasked");
    println!("      --mask C                        another character, or a token per masked run");
    println!("      -i [--explain] [--show-skipped] put each masking to you first; -U applies every one");
    println!("    trex templates [INPUT]...         each distinct record shape once with its count,");
    println!("                                      varying positions as <kind>; --pattern spells");
    println!("                                      it as a pattern; --rare [--cut N|P%] the rare ones");
    println!("      --record UNIT                   group records rather than lines, as a query does");
    println!("      --lib FILE|DIR                  declarations a --record-start or --record-span");
    println!("                                      pattern reads, with the ones scan takes inline");
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
    println!("      --hidden --no-ignore --binary   read what the walk skips, as a scan does");
    println!("      --sum C --avg C --min C --max C a column over capture C, each repeatable");
    println!("      --pN C                          the Nth percentile of C, N from 0 to 100: --p50, --p99");
    println!("      --percentile METHOD             nearest (the default), linear, lower or hybrid");
    println!("    trex index DIR...                 index a tree so a later scan opens only the");
    println!("                                      files that can match; --list reports one");
    println!("    trex tokens (FILE | --text STRING) each token as [start..end] kind \"text\";");
    println!("                                      --whitespace --json --binary; --lib FILE|DIR and");
    println!("                                      the declarations scan takes");
    println!("    trex escape TEXT                  the pattern that matches TEXT literally");
    println!("    trex lib [--json]                 the shipped library of named patterns");
    println!("    trex lib --test [FILE|DIR...]     run the test lines of pattern files, a directory");
    println!("                                      read as its .trex files, or the shipped library's");
    println!("                                      own where none is named");
    println!("    trex prefilter (FILE | --text STRING) [--literal L]...");
    println!("                                      report which literals might occur");
    println!("    trex grammar GRAMMAR (FILE | --text STRING) [--start RULE]");
    println!("                                      parse input against a token grammar");
    println!("                                      [--count|--best|--prob] semiring values");
    println!("                                      [--segment TEXT --dict FILE] parse over a");
    println!("                                      segmentation lattice of run-together text;");
    println!("                                      --list prints each tiling it accepts");
    println!("    trex bpe train CORPUS [--merges K] [--max-bytes N]");
    println!("                                      learn a subword (BPE) tokenizer from a corpus");
    println!("    trex bpe encode (FILE | --text S) --model M");
    println!("                                      segment text into learned subwords");
    println!();
    println!("AXES - one reading at every position of the input:");
    println!("    trex spectral (FILE | --text STRING)  filterbank/entropy/period/novelty + change-points");
    println!("                                      [--segment] change-point boundaries (the spectral subtokens)");
    println!("                                      [--bands] per-hop band table   [--timeline] texture timeline");
    println!("                                      [--classify] regions by texture and shape");
    println!("                                      [--json] machine-readable frames + boundaries");
    println!("                                      [--cp-threshold K] [--cp-floor F] [--cp-min-gap N]");
    println!("    trex seam (FILE | --text STRING)  bidirectional predictive segmentation: cut where the past");
    println!("                                      stops predicting the future; [--order K] [--field] [--segment]");
    println!("                                      [--english] [--passes N] [--cut-threshold K] [--json]");
    println!("                                      [--recover [FILE] [--compare-bpe]] word-boundary recovery vs count-BPE");
    println!("                                      [--grain-separation] function and block grain, synthetic stream");
    println!("    trex shape (FILE | --text STRING) token silhouettes, their period and template regions;");
    println!("                                      [--classes] [--period] [--segment] [--group G] [--json]");
    println!("                                      [--template-strength S]");
    println!("    trex orbit (FILE | --text STRING) each token under a symmetry group: [--group G]");
    println!("                                      [--collapse] [--boundary] [--same-as TEXT]");
    println!("    trex magnitude (FILE | --text STRING)  each token's order of magnitude, change and energy;");
    println!("                                      [--field] [--jumps] [--energy] [--top K] [--outliers]");
    println!("                                      [--jump-threshold K] [--outlier-sigma S]");
    println!("    trex stress (FILE | --text STRING)  bracket depth, strain and load; [--field] [--peaks]");
    println!("                                      [--fractures] [--peak-min-depth D] [--fracture-min-depth D]");
    println!("    trex flow (FILE | --text STRING)  a signal's slope, direction, momentum and reversals;");
    println!("                                      [--signal magnitude|stress|length] [--grain token|super]");
    println!("                                      [--field] [--reversals] [--analytic] [--window N] [--steady-band B]");
    println!("    trex observe (FILE | --text STRING)  where the reading behind a point and ahead of it disagree;");
    println!("                                      [--grain byte|token|super] [--field] [--contested]");
    println!("                                      [--contested-threshold T] [--contested-min-gap N]");
    println!("    trex gravity (FILE | --text STRING)  the pair field: each unit's strain, the bound of the");
    println!("                                      cut before it, and the gravity classes;");
    println!("                                      [--grain byte|token|super] [--field] [--strain] [--bound]");
    println!("                                      [--classes] [--top K]");
    println!("    trex context (FILE | --text STRING)  what each token is read against, as \\N{{>+1:F}} reads it;");
    println!("                                      [--fold window|unit|units|enclosing|echo|regime|phase|key]");
    println!("                                      [--field] [--period] [--agreement] [--token-window N]");
    println!("                                      [--unit-window N]");
    println!("    trex compress (FILE | --text STRING)  report achieved compression of the default coder");
    println!("                                      (logistic mix + orbit, auto-using the baked prior);");
    println!("                                      [--compare] full coder table  [--no-baked] corpus-free");
    println!("                                      [--chunks N] N-core chunked (0 = all cores)");
    println!("                                      [--gpu] the device coder  [--hybrid] CPU + GPU concurrent");
    println!("                                      in a build with the compress feature only:");
    println!("                                      cargo build --release --features compress");
    println!("    trex echo (FILE | --text STRING)  the recurrence axis: per-token echo count/lag/period,");
    println!("                                      novelty + echo rate; [--field] per-token detail");
    println!("                                      [--group case|shape|notation|ip|url|time|path|fold|numeric]");
    println!("                                      recurrence up to a symmetry or a representation");
    println!("                                      [--structure] recurring supertoken structures (structural rhyme)");
    println!("                                      [--top K] [--max-period-cv C]");
    println!("    trex relation (FILE | --text STRING)  the geometric tier: directed relations (encloses /");
    println!("                                      operator / adjacent), nesting load, holonomy, holography");
    println!("                                      (boundary reconstructs bulk), Ricci curvature, topology");
    println!("                                      (Betti numbers), geodesic shortcuts, entanglement min-cut;");
    println!("                                      [--edges] edges + chords  [--field] enclosure  [--canonical] canonical form");
    println!("    (every listing prints in full; --limit N bounds one, and anything cut is marked '+N more')");
    println!("    (every axis but spectral and seam reads tokens, and takes [--lib FILE|DIR]");
    println!("     [--shape|--shape-after|--kind|--let DECL] [--declare LINE]; observe and gravity at --grain");
    println!("     token or super)");
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
    println!("    \\V semver \\{{uuid}} uuid \\{{mac}} mac \\H hexcolor \\C cidr \\% percent \\Z bytesize \\$ money");
    println!("    \\D hash \\R duration \\L path");
    println!("    \\{{jwt}} \\{{creditcard}} \\{{base64}} \\{{hex}} \\{{geo}} \\{{phone}} \\{{quantity}}  by name only");
    println!("    anchors: ^ $ line start/end, \\A \\z input start/end, \\G where the previous match ended");
    println!();
    println!("AXIS PREDICATES & ANCHORS (match by computed property, not content):");
    println!("    \\M{{>6}}  magnitude (scale)        \\N{{>6}}  a number of that scale");
    println!("    \\N{{>+1}}  an order above the window before it   \\N{{>+2s}}  two sigmas above");
    println!("    \\N{{>+1:phase}}  above its column   \"key\":k \"=\" \\N{{>+1:k}}  above the key's history");
    println!("    \\F{{entropy>0.8}} \\F{{period}} \\F{{texture:code}}  spectral");
    println!("    =shape x  =case x  =notation x   fuzzy backreference (orbit)");
    println!("    #\"W(W,W)\"  silhouette (structural form)   !~\"lit\"  negative lookahead");
    println!("    @seam (predictive break)  @nested>k (deep nesting)  @ambiguous (vantage)");
    println!("    @seam:byte>1.5  @ambiguous>0.3  a cut of your own on either");
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
    println!("    trex echo server.log --structure");
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
    let mut paths_only = false;
    for arg in args {
        match arg.as_str() {
            path if paths_only || !is_flag(path) => tests.push(path.to_string()),
            "--" => paths_only = true,
            "-h" | "--help" => {
                println!("usage: trex lib [--json]");
                println!("       trex lib --test [FILE|DIR]...");
                println!("  the shipped library of named patterns: each entry's name, form, guard and what it matches");
                println!("  --json                 the entries as JSON");
                println!("  --test                 run the test lines of the pattern files named, a directory read as");
                println!("                         its .trex files, or the shipped library's own where none is named");
                println!("  a FILE that begins with a dash goes after --");
                return ExitCode::SUCCESS;
            }
            "--json" => json = true,
            "--test" => test = true,
            flag => return unknown_flag("lib", flag),
        }
    }
    if !test && !tests.is_empty() {
        eprintln!("trex lib: {} names declarations; --test runs its test lines", tests[0]);
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
/// checker a user's file goes through, and report them in that report's
/// form. It tells whether this build's kinds and their checks work, which a
/// user of a binary they did not build has no other way to check.
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
/// flags do, a directory as every `.trex` file under it in path order and a
/// file named twice read once, and run every `test` line they hold against
/// what the set finally declares. Each failure prints as a `FILE:LINE:`
/// line, then one line per file counts its tests; the exit status is a
/// failure when any test fails.
fn run_lib_tests(paths: &[String]) -> ExitCode {
    let mut decls = trex::Declarations::new();
    if let Err(e) = decls.include(paths) {
        eprintln!("trex lib: {e}");
        return ExitCode::FAILURE;
    }
    let shapes = decls.set();
    let failures = shapes.run_tests();
    for failure in &failures {
        println!("{failure}");
    }
    let mut any_failed = false;
    for file in &decls.files() {
        let path = file.display();
        let file = Some(file.as_path());
        let total = shapes.tests().iter().filter(|t| t.file.as_deref() == file).count();
        let failed: std::collections::BTreeSet<usize> =
            failures.iter().filter(|f| f.file.as_deref() == file).map(|f| f.line).collect();
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
    const COMMAND: &str = "grammar";
    const USAGE: &str = "usage: trex grammar GRAMMAR (FILE | --text STRING | --segment TEXT --dict FILE) [--grammar-text SRC] [--start RULE] [--count | --best | --prob | --list]";
    if args.is_empty() {
        eprintln!("{USAGE}");
        return ExitCode::FAILURE;
    }
    let mut grammar_src: Option<String> = None;
    let mut input: Option<Vec<u8>> = None;
    let mut start: Option<String> = None;
    let mut count = false;
    let mut best = false;
    let mut prob = false;
    let mut list = false;
    let mut segment: Option<String> = None;
    let mut dict_path: Option<String> = None;

    // The first positional is the grammar file unless --grammar-text is
    // given, and the next the input.
    let mut i = 0;
    let mut paths_only = false;
    while i < args.len() {
        let arg = args[i].as_str();
        if paths_only || !is_flag(arg) {
            if grammar_src.is_none() {
                match std::fs::read_to_string(arg) {
                    Ok(s) => grammar_src = Some(s),
                    Err(e) => {
                        eprintln!("trex: cannot read grammar {arg}: {e}");
                        return ExitCode::FAILURE;
                    }
                }
            } else {
                match read_input(arg) {
                    Ok(b) => input = Some(b),
                    Err(e) => {
                        eprintln!("trex: cannot read {arg}: {e}");
                        return ExitCode::FAILURE;
                    }
                }
            }
            i += 1;
            continue;
        }
        match arg {
            "--" => paths_only = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                println!("  parse the input against a token grammar");
                println!("  --count                the number of derivations");
                println!("  --best                 the most probable derivation's probability");
                println!("  --prob                 the total probability over every derivation");
                println!("  --segment TEXT --dict FILE  parse over the segmentations of run-together text;");
                println!("                         --list prints each tiling the grammar accepts");
                println!("  a FILE that begins with a dash goes after --");
                return ExitCode::SUCCESS;
            }
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
            "--list" => list = true,
            "--segment" => {
                i += 1;
                segment = args.get(i).cloned();
            }
            "--dict" => {
                i += 1;
                dict_path = args.get(i).cloned();
            }
            flag => return unknown_flag(COMMAND, flag),
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
    if list && segment.is_none() {
        eprintln!("trex grammar: --list lists the tilings --segment reads; give --segment TEXT --dict FILE");
        return ExitCode::FAILURE;
    }
    if list && (count || best || prob) {
        eprintln!("trex grammar: --list, --count, --best and --prob each say what is printed; give one");
        return ExitCode::FAILURE;
    }

    // Segmentation lattice: parse a run-together string over a dictionary,
    // so the grammar disambiguates the tokenization. --count reports the
    // segmentations the grammar accepts, --best the most probable, and --list
    // each tiling with its best parse's probability and its parse count.
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
        if list {
            let tilings = grammar.segment(&seg, &dict);
            for t in &tilings {
                println!("{}\t{}\t{}", t.words.join(" "), t.probability, t.parses);
            }
            return if tilings.is_empty() { ExitCode::FAILURE } else { ExitCode::SUCCESS };
        }
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

/// The usage `trex bpe` prints.
const BPE_USAGE: &str =
    "usage: trex bpe (train CORPUS [--merges K] [--max-bytes N] | encode (--text STRING | FILE) --model MODEL)";

/// Print what `trex bpe --help` shows.
fn print_bpe_help() {
    println!("{BPE_USAGE}");
    println!("  train learns a subword (BPE) tokenizer from a corpus and prints its merges:");
    println!("        --merges K of them (1000 by default), from the first N bytes with --max-bytes N");
    println!("  encode segments text into the subwords of a learned --model");
    println!("  a FILE that begins with a dash goes after --");
}

fn run_bpe(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("train") => run_bpe_train(&args[1..]),
        Some("encode") => run_bpe_encode(&args[1..]),
        Some("-h" | "--help") => {
            print_bpe_help();
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{BPE_USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn run_bpe_train(args: &[String]) -> ExitCode {
    let mut corpus_path: Option<String> = None;
    let mut merges = 1000usize;
    let mut max_bytes: Option<usize> = None;
    let mut i = 0;
    let mut paths_only = false;
    while i < args.len() {
        match args[i].as_str() {
            path if paths_only || !is_flag(path) => {
                if corpus_path.is_some() {
                    eprintln!("trex bpe train: unexpected argument {path}");
                    return ExitCode::FAILURE;
                }
                corpus_path = Some(path.to_string());
            }
            "--" => paths_only = true,
            "-h" | "--help" => {
                print_bpe_help();
                return ExitCode::SUCCESS;
            }
            "--merges" => {
                i += 1;
                merges = flag_value_or_fail!("bpe train", args, i, "--merges");
            }
            "--max-bytes" => {
                i += 1;
                max_bytes = Some(flag_value_or_fail!("bpe train", args, i, "--max-bytes"));
            }
            flag => return unknown_flag("bpe train", flag),
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
    let mut paths_only = false;
    while i < args.len() {
        match args[i].as_str() {
            path if paths_only || !is_flag(path) => {
                if text.is_some() {
                    eprintln!("trex bpe encode: unexpected argument {path}");
                    return ExitCode::FAILURE;
                }
                match std::fs::read(path) {
                    Ok(b) => text = Some(b),
                    Err(e) => {
                        eprintln!("trex: cannot read {path}: {e}");
                        return ExitCode::FAILURE;
                    }
                }
            }
            "--" => paths_only = true,
            "-h" | "--help" => {
                print_bpe_help();
                return ExitCode::SUCCESS;
            }
            "--text" => {
                i += 1;
                text = args.get(i).map(|s| s.clone().into_bytes());
            }
            "--model" => {
                i += 1;
                model_path = args.get(i).cloned();
            }
            flag => return unknown_flag("bpe encode", flag),
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
    const COMMAND: &str = "observe";
    let mut input: Option<Vec<u8>> = None;
    let mut show_field = false;
    let mut show_contested = false;
    let mut grain = trex::ast::Grain::Byte;
    let mut cfg = trex::observation::ObservationConfig::default();
    let mut gap_given = false;
    let mut decls = trex::Declarations::new();
    let mut i = 0;
    let mut paths_only = false;
    while i < args.len() {
        let arg = args[i].as_str();
        if paths_only || !is_flag(arg) {
            match read_input(arg) {
                Ok(b) => input = Some(b),
                Err(e) => {
                    eprintln!("trex: cannot read {arg}: {e}");
                    return ExitCode::FAILURE;
                }
            }
            i += 1;
            continue;
        }
        take_declaration_or_fail!(args, i, decls);
        match arg {
            "--" => paths_only = true,
            "-h" | "--help" => {
                print_axis_help(COMMAND);
                return ExitCode::SUCCESS;
            }
            "--field" => show_field = true,
            "--contested" => show_contested = true,
            "--contested-threshold" => {
                i += 1;
                cfg.contested_threshold = cut_value_or_fail!("observe", args, i, "--contested-threshold");
                if !(0.0..=1.0).contains(&cfg.contested_threshold) {
                    eprintln!(
                        "trex observe: --contested-threshold {} names no disagreement: a disagreement runs 0 to 1",
                        cfg.contested_threshold
                    );
                    return ExitCode::FAILURE;
                }
            }
            "--contested-min-gap" => {
                i += 1;
                cfg.contested_min_gap = flag_value_or_fail!("observe", args, i, "--contested-min-gap");
                gap_given = true;
            }
            "--grain" => {
                i += 1;
                grain = match args.get(i).map(String::as_str).map(trex::ast::Grain::parse) {
                    Some(Some(g)) => g,
                    Some(None) => {
                        eprintln!("trex observe: unknown grain '{}' (byte|token|super)", args[i]);
                        return ExitCode::FAILURE;
                    }
                    None => {
                        eprintln!("trex observe: --grain needs a grain (byte|token|super)");
                        return ExitCode::FAILURE;
                    }
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
            flag => return unknown_flag(COMMAND, flag),
        }
        i += 1;
    }
    let Some(input) = input else {
        eprintln!("trex observe: no input given (pass a FILE or --text STRING)");
        return ExitCode::FAILURE;
    };
    use trex::ast::Grain;
    use trex::observation;
    if gap_given && grain != Grain::Byte {
        eprintln!(
            "trex observe: --contested-min-gap spaces the byte grain's contested points; the token and super grains keep every local maximum"
        );
        return ExitCode::FAILURE;
    }
    if grain == Grain::Byte && !decls.set().is_empty() {
        eprintln!("trex observe: declarations decide how tokens are read, and the byte grain reads bytes; --grain token or super reads them");
        return ExitCode::FAILURE;
    }
    // Each unit's byte span: one byte, one significant token or one supertoken.
    let (f, spans, unit): (observation::ObservationField, Vec<(usize, usize)>, &str) = match grain {
        Grain::Byte => (observation::analyze_with(&input, &cfg), (0..input.len()).map(|b| (b, b + 1)).collect(), "byte"),
        Grain::Token => {
            let sig = lex_declared(&input, &decls, true);
            (observation::analyze_tokens(&sig, &cfg), sig.iter().map(|t| (t.start(), t.end())).collect(), "token")
        }
        Grain::Super => {
            let units = trex::supertoken::supertokens_from(&lex_declared(&input, &decls, false), &input);
            (observation::analyze_supertokens(&units, &cfg), units.iter().map(|u| (u.start, u.end)).collect(), "supertoken")
        }
    };
    match grain {
        Grain::Byte => println!("trex observe: {} bytes, {} contested point(s)", input.len(), f.contested.len()),
        Grain::Token | Grain::Super => println!(
            "trex observe ({} grain): {} bytes, {} {unit}s, {} contested point(s)",
            grain.name(),
            input.len(),
            spans.len(),
            f.contested.len()
        ),
    }
    if let Some(&idx) = f.contested.iter().max_by(|&&a, &&b| {
        f.frames[a]
            .disagreement
            .partial_cmp(&f.frames[b].disagreement)
            .unwrap_or(std::cmp::Ordering::Equal)
    }) {
        println!(
            "  peak observer-dependence: {:.2} at byte {}",
            f.frames[idx].disagreement,
            spans[idx].0
        );
    }
    if show_contested {
        println!("  contested points ({}):", f.contested.len());
        for &idx in &f.contested {
            let b = spans[idx].0;
            let lo = b.saturating_sub(10);
            let preview = String::from_utf8_lossy(&input[lo..(b + 14).min(input.len())])
                .replace(['\n', '\t'], " ");
            println!("    @{b:>6}  ...{preview}...");
        }
    }
    if show_field {
        let n = f.frames.len();
        let hop = (n / 24).max(1);
        println!("  vantage (causal | anticausal | centered | disagree), every {hop} {unit}s:");
        let mut t = 0;
        while t < n {
            let fr = &f.frames[t];
            let (s, e) = spans[t];
            match grain {
                Grain::Byte => println!(
                    "    @{s:>6}  c={:.2} a={:.2} m={:.2} |d|={:.2}",
                    fr.causal, fr.anticausal, fr.centered, fr.disagreement
                ),
                Grain::Token | Grain::Super => {
                    let w = String::from_utf8_lossy(&input[s..e]).replace(['\n', '\t'], " ");
                    println!(
                        "    @{s:>6}  {:<14} c={:.2} a={:.2} m={:.2} |d|={:.2}",
                        format!("{w:.14}"),
                        fr.causal,
                        fr.anticausal,
                        fr.centered,
                        fr.disagreement
                    );
                }
            }
            t += hop;
        }
    }
    ExitCode::SUCCESS
}

fn run_gravity(args: &[String]) -> ExitCode {
    const COMMAND: &str = "gravity";
    let mut input: Option<Vec<u8>> = None;
    let mut grain = trex::ast::Grain::Token;
    let mut show_field = false;
    let mut show_strain = false;
    let mut show_bound = false;
    let mut show_classes = false;
    let mut top = 8usize;
    let mut limit = usize::MAX;
    let mut decls = trex::Declarations::new();
    let mut i = 0;
    let mut paths_only = false;
    while i < args.len() {
        let arg = args[i].as_str();
        if paths_only || !is_flag(arg) {
            match read_input(arg) {
                Ok(b) => input = Some(b),
                Err(e) => {
                    eprintln!("trex: cannot read {arg}: {e}");
                    return ExitCode::FAILURE;
                }
            }
            i += 1;
            continue;
        }
        take_declaration_or_fail!(args, i, decls);
        match arg {
            "--" => paths_only = true,
            "-h" | "--help" => {
                print_axis_help(COMMAND);
                return ExitCode::SUCCESS;
            }
            "--field" => show_field = true,
            "--strain" => show_strain = true,
            "--bound" => show_bound = true,
            "--classes" => show_classes = true,
            "--top" => {
                i += 1;
                top = flag_value_or_fail!("gravity", args, i, "--top");
            }
            "--limit" => {
                i += 1;
                limit = flag_value_or_fail!("gravity", args, i, "--limit");
            }
            "--grain" => {
                i += 1;
                grain = match args.get(i).map(String::as_str).map(trex::ast::Grain::parse) {
                    Some(Some(g)) => g,
                    Some(None) => {
                        eprintln!("trex gravity: unknown grain '{}' (byte|token|super)", args[i]);
                        return ExitCode::FAILURE;
                    }
                    None => {
                        eprintln!("trex gravity: --grain needs a grain (byte|token|super)");
                        return ExitCode::FAILURE;
                    }
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
            flag => return unknown_flag(COMMAND, flag),
        }
        i += 1;
    }
    let Some(input) = input else {
        eprintln!("trex gravity: no input given (pass a FILE or --text STRING)");
        return ExitCode::FAILURE;
    };
    use trex::ast::Grain;
    if grain == Grain::Byte && !decls.set().is_empty() {
        eprintln!(
            "trex gravity: declarations decide how tokens are read, and the byte grain reads bytes; --grain token or super reads them"
        );
        return ExitCode::FAILURE;
    }
    let toks = if grain == Grain::Byte { Vec::new() } else { lex_declared(&input, &decls, false) };
    let r = trex::gravity::Readings::read(grain, &input, &toks);
    let unit = match grain {
        Grain::Byte => "byte",
        Grain::Token => "token",
        Grain::Super => "supertoken",
    };
    // A unit as a reader would name it: a byte by its label, a token or a
    // supertoken by its text on one line.
    let named = |u: usize| -> String {
        match grain {
            Grain::Byte => r.type_label(r.type_of(u)),
            Grain::Token | Grain::Super => {
                let (s, e) = r.span_of(u);
                String::from_utf8_lossy(&input[s..e]).replace(['\n', '\t', '\r'], " ")
            }
        }
    };
    // A type named in a sentence or a list, in quotes: a byte as a character
    // literal, escaped as `u8::escape_ascii` escapes it with a double quote
    // left bare, and a token's or a supertoken's key by its label.
    let quoted_type = |ty: u32| -> String {
        match grain {
            Grain::Byte => match u8::try_from(ty).expect("a byte-grain type is a byte") {
                b'"' => "'\"'".to_string(),
                b => format!("'{}'", b.escape_ascii()),
            },
            Grain::Token | Grain::Super => format!("'{}'", r.type_label(ty)),
        }
    };
    // A unit named in a sentence: a byte as its quoted type, and a token's or
    // a supertoken's text in quotes.
    let quoted = |u: usize| -> String {
        match grain {
            Grain::Byte => quoted_type(r.type_of(u)),
            Grain::Token | Grain::Super => format!("'{}'", named(u)),
        }
    };
    let classes = r.classes();
    let strained = r.most_strained(top);
    let weakest = r.weakest_cuts(top);
    match grain {
        Grain::Byte => println!(
            "trex gravity (byte grain): {} bytes, {} types, {} gravity class(es)",
            input.len(),
            r.types(),
            classes.len()
        ),
        Grain::Token | Grain::Super => println!(
            "trex gravity ({} grain): {} bytes, {} {unit}s, {} types, {} gravity class(es)",
            grain.name(),
            input.len(),
            r.len(),
            r.types(),
            classes.len()
        ),
    }
    if let Some(&u) = strained.first() {
        let reading = r.reading_of(u);
        if let (Some(s), Some(p)) = (reading.strain, reading.strain_percentile) {
            println!("  most strain: {s:.2} bits at {} (byte {}, percentile {p:.0})", quoted(u), r.start_of(u));
        }
    }
    if let Some(&u) = weakest.first() {
        let reading = r.reading_of(u);
        if let (Some(b), Some(p)) = (reading.bound, reading.bound_percentile) {
            println!(
                "  weakest cut: {b:.2} bits per pair before {} (byte {}, percentile {p:.0})",
                quoted(u),
                r.start_of(u)
            );
        }
    }
    let with_reading = r.len().saturating_sub(1);
    if show_strain {
        println!("  most strained ({} of {with_reading} {unit}s with a reading):", strained.len());
        for &u in &strained {
            let reading = r.reading_of(u);
            if let (Some(s), Some(p)) = (reading.strain, reading.strain_percentile) {
                println!("    @{:>6}  {:<14} {s:>7.2} bits  percentile {p:>3.0}", r.start_of(u), format!("{:.14}", named(u)));
            }
        }
    }
    if show_bound {
        println!("  weakest cuts ({} of {with_reading}), each before the {unit} named:", weakest.len());
        for &u in &weakest {
            let reading = r.reading_of(u);
            if let (Some(b), Some(p)) = (reading.bound, reading.bound_percentile) {
                println!(
                    "    @{:>6}  {:<14} {b:>7.2} bits per pair  percentile {p:>3.0}",
                    r.start_of(u),
                    format!("{:.14}", named(u))
                );
            }
        }
    }
    if show_classes {
        println!("  gravity classes ({}):", classes.len());
        for g in &classes {
            let types: Vec<String> = g.types.iter().map(|&t| quoted_type(t)).collect();
            println!("    class {:>2}: {} {unit}s of {}", g.class, g.units, types.join(", "));
        }
    }
    if show_field {
        println!(
            "  per-{unit} (offset, {unit}, strain in bits and its percentile, bound in bits per pair and its percentile, class):"
        );
        let shown = r.len().min(limit);
        for u in 0..shown {
            let reading = r.reading_of(u);
            let pair = |v: Option<f32>, p: Option<f64>| match (v, p) {
                (Some(v), Some(p)) => format!("{v:>7.2} {p:>5.0}"),
                _ => format!("{:>7} {:>5}", "-", "-"),
            };
            let class = match r.class_of(r.type_of(u)) {
                Some(c) => c.to_string(),
                None => "-".to_string(),
            };
            println!(
                "    @{:>6}  {:<14} {}  {}  {class:>2}",
                r.start_of(u),
                format!("{:.14}", named(u)),
                pair(reading.strain, reading.strain_percentile),
                pair(reading.bound, reading.bound_percentile)
            );
        }
        if r.len() > shown {
            println!("    ... (+{} more {unit}s; raise --limit)", r.len() - shown);
        }
    }
    ExitCode::SUCCESS
}

fn run_context(args: &[String]) -> ExitCode {
    const COMMAND: &str = "context";
    let mut input: Option<Vec<u8>> = None;
    let mut fold = trex::context::Fold::Window;
    let mut show_field = false;
    let mut show_period = false;
    let mut show_agreement = false;
    let mut cfg = trex::context::ContextConfig::default();
    let mut limit = usize::MAX;
    let mut decls = trex::Declarations::new();
    let mut i = 0;
    let mut paths_only = false;
    while i < args.len() {
        let arg = args[i].as_str();
        if paths_only || !is_flag(arg) {
            match read_input(arg) {
                Ok(b) => input = Some(b),
                Err(e) => {
                    eprintln!("trex: cannot read {arg}: {e}");
                    return ExitCode::FAILURE;
                }
            }
            i += 1;
            continue;
        }
        take_declaration_or_fail!(args, i, decls);
        match arg {
            "--" => paths_only = true,
            "-h" | "--help" => {
                print_axis_help(COMMAND);
                return ExitCode::SUCCESS;
            }
            "--field" => show_field = true,
            "--period" => show_period = true,
            "--agreement" => show_agreement = true,
            "--fold" => {
                i += 1;
                let names = trex::context::Fold::NAMES.join("|");
                fold = match args.get(i).map(String::as_str).map(trex::context::Fold::parse) {
                    Some(Some(f)) => f,
                    Some(None) => {
                        eprintln!("trex context: unknown fold '{}' ({names})", args[i]);
                        return ExitCode::FAILURE;
                    }
                    None => {
                        eprintln!("trex context: --fold needs a fold ({names})");
                        return ExitCode::FAILURE;
                    }
                };
            }
            "--token-window" => {
                i += 1;
                cfg.token_window = flag_value_or_fail!("context", args, i, "--token-window");
            }
            "--unit-window" => {
                i += 1;
                cfg.unit_window = flag_value_or_fail!("context", args, i, "--unit-window");
            }
            "--limit" => {
                i += 1;
                limit = flag_value_or_fail!("context", args, i, "--limit");
            }
            "--text" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --text needs a value");
                    return ExitCode::FAILURE;
                };
                input = Some(v.clone().into_bytes());
            }
            flag => return unknown_flag(COMMAND, flag),
        }
        i += 1;
    }
    let Some(input) = input else {
        eprintln!("trex context: no input given (pass a FILE or --text STRING)");
        return ExitCode::FAILURE;
    };
    if cfg.token_window == 0 || cfg.unit_window == 0 {
        eprintln!("trex context: a window folds at least one unit; --token-window and --unit-window take 1 or more");
        return ExitCode::FAILURE;
    }
    use trex::context::Fold;
    let toks = lex_declared(&input, &decls, false);
    let c = trex::context::Contexts::read(&toks, &input, &cfg);
    let sig: Vec<usize> = (0..toks.len()).filter(|&i| toks[i].is_significant()).collect();
    let text_of = |s: usize, e: usize| String::from_utf8_lossy(&input[s..e]).replace(['\n', '\t', '\r'], " ");
    println!(
        "trex context ({} fold): {} bytes, {} tokens, {} supertokens",
        fold.name(),
        input.len(),
        sig.len(),
        c.supers.units.len()
    );
    match c.related.period {
        Some(p) => {
            let live: Vec<String> = c.live_periods.iter().map(u16::to_string).collect();
            println!("  record period: {p} tokens (live periods {})", live.join(", "));
        }
        None => println!("  record period: none"),
    }
    let (regime, shape, seam) = c.field.alignment();
    println!(
        "  alignment: regime {regime:.2}, shape {shape:.2}, seam {seam:.2} of supertokens at their role's usual offset"
    );
    if show_period {
        println!("  per-token column of the record period:");
        let shown = sig.len().min(limit);
        for &t in &sig[..shown] {
            let column = match c.phase_of(t) {
                Some(p) => p.to_string(),
                None => "-".to_string(),
            };
            println!("    @{:>6}  {:<14} {column:>3}", toks[t].start(), format!("{:.14}", text_of(toks[t].start(), toks[t].end())));
        }
        if sig.len() > shown {
            println!("    ... (+{} more tokens; raise --limit)", sig.len() - shown);
        }
    }
    if show_agreement {
        println!(
            "  per-supertoken (role, then the nearest spectral change point, shape change point and seam cut, in tokens from its start):"
        );
        let shown = c.field.agreement.len().min(limit);
        for (u, a) in c.field.agreement.iter().enumerate().take(shown) {
            let unit = &c.supers.units[u];
            let off = |o: Option<i32>| match o {
                Some(o) => format!("{o:>4}"),
                None => format!("{:>4}", "-"),
            };
            println!(
                "    @{:>6}  {:<14} {:<8} {} {} {}",
                unit.start,
                format!("{:.14}", text_of(unit.start, unit.end)),
                a.role.label(),
                off(a.regime),
                off(a.shape),
                off(a.seam)
            );
        }
        if c.field.agreement.len() > shown {
            println!("    ... (+{} more supertokens; raise --limit)", c.field.agreement.len() - shown);
        }
    }
    if show_field {
        let what = match fold {
            Fold::Window => "the window of tokens before each",
            Fold::Unit => "the supertoken holding each",
            Fold::Units => "the window of supertokens ending at the one holding each",
            Fold::Enclosing => "the heads of the brackets enclosing each",
            Fold::Echo => "the earlier occurrences of each one's key",
            Fold::Regime => "the tokens since the last spectral change point",
            Fold::Phase => "the earlier tokens at each one's column of the record period",
            Fold::Key => "the values each one's key was bound to before",
        };
        println!("  per-token context, {what}:");
        let shown = sig.len().min(limit);
        for &t in &sig[..shown] {
            let p = c.at(fold, t);
            let over = p.magnitude.count;
            println!(
                "    @{:>6}  {:<14} over {over}",
                toks[t].start(),
                format!("{:.14}", text_of(toks[t].start(), toks[t].end()))
            );
            if over == 0 {
                continue;
            }
            println!(
                "        magnitude: mean {:.2}, spread {:.2}, max {:.2}",
                p.magnitude.mean(),
                p.magnitude.std_dev(),
                p.magnitude.max
            );
            println!("        stress: mean {:.2}, max {}", p.stress.mean_depth(), p.stress.max_depth);
            println!(
                "        spectral: entropy {:.2}, period {}, strength {:.2}",
                p.spectral.mean_entropy(),
                p.spectral.period,
                p.spectral.period_strength
            );
            println!("        echo: novel {}, echoed {}", p.echo.novel, p.echo.echoed);
            println!(
                "        observation: causal {:.2}, anticausal {:.2}, centered {:.2}, disagreement {:.2}, disagreement max {:.2}, contested {}",
                p.observation.mean_causal(),
                p.observation.mean_anticausal(),
                p.observation.mean_centered(),
                p.observation.mean_disagreement(),
                p.observation.disagreement_max,
                p.observation.contested
            );
            println!(
                "        seam: forward {:.2}, backward {:.2}, strength {:.2}, strength max {:.2}, cuts {}",
                p.seam.mean_fwd(),
                p.seam.mean_bwd(),
                p.seam.mean_strength(),
                p.seam.strength_max,
                p.seam.cuts
            );
            println!(
                "        flow: slope {:.2}, rising {}, falling {}, steady {}, momentum {}, reversals {}",
                p.flow.mean_slope(),
                p.flow.rising,
                p.flow.falling,
                p.flow.steady,
                p.flow.momentum_max,
                p.flow.reversals
            );
        }
        if sig.len() > shown {
            println!("    ... (+{} more tokens; raise --limit)", sig.len() - shown);
        }
    }
    ExitCode::SUCCESS
}

fn run_flow(args: &[String]) -> ExitCode {
    const COMMAND: &str = "flow";
    let mut input: Option<Vec<u8>> = None;
    let mut over = trex::flow::Signal::Magnitude;
    let mut show_field = false;
    let mut show_reversals = false;
    let mut show_analytic = false;
    let mut by_super = false;
    let mut cfg = trex::flow::FlowConfig::default();
    let mut limit = usize::MAX;
    let mut decls = trex::Declarations::new();
    let mut i = 0;
    let mut paths_only = false;
    while i < args.len() {
        let arg = args[i].as_str();
        if paths_only || !is_flag(arg) {
            match read_input(arg) {
                Ok(b) => input = Some(b),
                Err(e) => {
                    eprintln!("trex: cannot read {arg}: {e}");
                    return ExitCode::FAILURE;
                }
            }
            i += 1;
            continue;
        }
        take_declaration_or_fail!(args, i, decls);
        match arg {
            "--" => paths_only = true,
            "-h" | "--help" => {
                print_axis_help(COMMAND);
                return ExitCode::SUCCESS;
            }
            "--field" => show_field = true,
            "--reversals" => show_reversals = true,
            "--analytic" => show_analytic = true,
            "--steady-band" => {
                i += 1;
                cfg.steady_band = cut_value_or_fail!("flow", args, i, "--steady-band");
            }
            "--window" => {
                i += 1;
                cfg.window = flag_value_or_fail!("flow", args, i, "--window");
            }
            "--grain" => {
                i += 1;
                by_super = match args.get(i).map(String::as_str) {
                    Some("token") => false,
                    Some("super") => true,
                    Some(other) => {
                        eprintln!("trex flow: unknown grain '{other}' (token|super)");
                        return ExitCode::FAILURE;
                    }
                    None => {
                        eprintln!("trex flow: --grain needs a grain (token|super)");
                        return ExitCode::FAILURE;
                    }
                };
            }
            "--limit" => {
                i += 1;
                limit = flag_value_or_fail!("flow", args, i, "--limit");
            }
            "--signal" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --signal needs a signal (magnitude|stress|length)");
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
            flag => return unknown_flag(COMMAND, flag),
        }
        i += 1;
    }
    let Some(input) = input else {
        eprintln!("trex flow: no input given (pass a FILE or --text STRING)");
        return ExitCode::FAILURE;
    };
    use trex::flow;
    let (f, analytic, unit) = if by_super {
        let toks = lex_declared(&input, &decls, false);
        let f = flow::analyze_supertokens_with(&toks, &input, over, &cfg);
        let analytic = show_analytic.then(|| flow::analytic_supertokens(&toks, &input, over));
        (f, analytic, "supertoken")
    } else {
        let toks = lex_declared(&input, &decls, true);
        let f = flow::analyze_with(&toks, &input, over, &cfg);
        let analytic = show_analytic.then(|| flow::analytic(&toks, &input, over));
        (f, analytic, "token")
    };
    println!(
        "trex flow (signal {}): {} bytes, {} {unit}s, {} reversal(s)",
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
        println!("  per-{unit} ({unit} -> slope | direction | momentum):");
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
        if f.frames.len() > limit {
            println!("    ... (+{} more {unit}s; raise --limit)", f.frames.len() - limit);
        }
    }
    if let Some(a) = &analytic {
        // A unit reads the signal `latency` units past it, so the last
        // `latency` units have no full reading and print a dash.
        let read = f.n_tokens.saturating_sub(a.latency);
        println!("  analytic ({unit} -> amplitude | phase | frequency), latency {} {unit}s:", a.latency);
        for k in 0..f.n_tokens.min(limit) {
            let (s, e) = f.spans[k];
            let w = String::from_utf8_lossy(&input[s..e]).replace('\n', "\\n");
            if k < read {
                println!(
                    "    {:<14} amp={:>6.2} phase={:>+6.2} freq={:>+6.2}",
                    format!("{w:.14}"),
                    a.amplitude[k],
                    a.phase[k],
                    a.frequency[k]
                );
            } else {
                println!("    {:<14} amp={:>6} phase={:>6} freq={:>6}", format!("{w:.14}"), "-", "-", "-");
            }
        }
        if f.n_tokens > limit {
            println!("    ... (+{} more {unit}s; raise --limit)", f.n_tokens - limit);
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
    const COMMAND: &str = "echo";
    let mut input: Option<Vec<u8>> = None;
    let mut show_field = false;
    let mut show_structure = false;
    let mut orbit = trex::OrbitGroup::Identity;
    let mut max_period_cv = trex::echo::EchoConfig::default().max_period_cv;
    let mut top = 8usize;
    // --field prints the whole field, and only --limit N bounds it.
    let mut limit = usize::MAX;
    let mut decls = trex::Declarations::new();
    let mut i = 0;
    let mut paths_only = false;
    while i < args.len() {
        let arg = args[i].as_str();
        if paths_only || !is_flag(arg) {
            match read_input(arg) {
                Ok(b) => input = Some(b),
                Err(e) => {
                    eprintln!("trex: cannot read {arg}: {e}");
                    return ExitCode::FAILURE;
                }
            }
            i += 1;
            continue;
        }
        take_declaration_or_fail!(args, i, decls);
        match arg {
            "--" => paths_only = true,
            "-h" | "--help" => {
                print_axis_help(COMMAND);
                return ExitCode::SUCCESS;
            }
            "--field" => show_field = true,
            "--structure" => show_structure = true,
            "--max-period-cv" => {
                i += 1;
                max_period_cv = cut_value_or_fail!("echo", args, i, "--max-period-cv");
            }
            "--top" => {
                i += 1;
                top = flag_value_or_fail!("echo", args, i, "--top");
            }
            "--limit" => {
                i += 1;
                limit = flag_value_or_fail!("echo", args, i, "--limit");
            }
            "--group" => {
                i += 1;
                let Some(name) = args.get(i) else {
                    eprintln!("trex echo: --group needs a value");
                    return ExitCode::FAILURE;
                };
                let Some(g) = trex::OrbitGroup::parse(name) else {
                    eprintln!(
                        "trex echo: unknown group '{name}' (use identity, case, shape, notation, e8, ip, url, time, path, fold, numeric, or a typed relation such as subnet/24, domain or day)"
                    );
                    return ExitCode::FAILURE;
                };
                orbit = g;
            }
            "--text" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --text needs a value");
                    return ExitCode::FAILURE;
                };
                input = Some(v.clone().into_bytes());
            }
            flag => return unknown_flag(COMMAND, flag),
        }
        i += 1;
    }
    let Some(input) = input else {
        eprintln!("trex echo: no input given (pass a FILE or --text STRING)");
        return ExitCode::FAILURE;
    };
    use trex::echo;
    let toks = lex_declared(&input, &decls, false);
    let cfg = echo::EchoConfig { orbit, max_period_cv, ..Default::default() };
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
        let keyed: Vec<_> = f.frames.iter().filter(|fr| fr.keyed).collect();
        for fr in keyed.iter().take(limit) {
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
        if keyed.len() > limit {
            println!("    ... (+{} more tokens; raise --limit)", keyed.len() - limit);
        }
    }
    if show_structure {
        let rhymes = echo::analyze_super_tokens(&toks, &input, &cfg);
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
    const COMMAND: &str = "relation";
    let mut input: Option<Vec<u8>> = None;
    let mut show_edges = false;
    let mut show_field = false;
    let mut show_canonical = false;
    let mut limit = usize::MAX;
    let mut decls = trex::Declarations::new();
    let mut i = 0;
    let mut paths_only = false;
    while i < args.len() {
        let arg = args[i].as_str();
        if paths_only || !is_flag(arg) {
            match read_input(arg) {
                Ok(b) => input = Some(b),
                Err(e) => {
                    eprintln!("trex: cannot read {arg}: {e}");
                    return ExitCode::FAILURE;
                }
            }
            i += 1;
            continue;
        }
        take_declaration_or_fail!(args, i, decls);
        match arg {
            "--" => paths_only = true,
            "-h" | "--help" => {
                print_axis_help(COMMAND);
                return ExitCode::SUCCESS;
            }
            "--edges" => show_edges = true,
            "--field" => show_field = true,
            "--canonical" => show_canonical = true,
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
            flag => return unknown_flag(COMMAND, flag),
        }
        i += 1;
    }
    let Some(input) = input else {
        eprintln!("trex relation: no input given (pass a FILE or --text STRING)");
        return ExitCode::FAILURE;
    };
    use trex::relation::{self, RelationKind};
    let toks = lex_declared(&input, &decls, false);
    let f = relation::analyze(&toks, &input);
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
    let h = trex::holography::analyze(&toks, &input);
    let rebuilt = h.reconstruct_bulk(&toks);
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
    let curv = trex::curvature::analyze(&toks, &input);
    println!(
        "  curvature: min {} (sharpest bottleneck), mean {:.2}, {} bridge edge(s)",
        curv.min_ricci(),
        curv.mean_ricci(),
        curv.bridges()
    );
    let topo = trex::topology::analyze(&toks, &input);
    println!(
        "  topology: {} nodes, {} edges, b0 {} component(s), b1 {} independent loop(s), euler {}",
        topo.nodes,
        topo.edges,
        topo.components,
        topo.cycle_rank,
        topo.euler()
    );
    let geo = trex::geodesic::analyze(&toks, &input);
    println!(
        "  geodesic: longest reuse shortcut collapses a {}-token span to one hop",
        geo.max_shortcut()
    );
    let ent = trex::entanglement::analyze(&toks, &input);
    match ent.min_cut() {
        Some((k, c)) => println!(
            "  entanglement: peak {}, minimal cut {} crossing(s) before token {k}",
            ent.max_entanglement(),
            c
        ),
        None => println!("  entanglement: peak {}", ent.max_entanglement()),
    }
    if show_canonical {
        println!("  canonical form (alpha-equivalence):");
        println!("    {}", trex::gauge::canonicalize(&toks, &input));
    }
    let word = |i: usize| String::from_utf8_lossy(&input[f.spans[i].0..f.spans[i].1]).into_owned();
    if show_edges {
        let named = |k: RelationKind, label: &str| {
            let edges: Vec<_> = f.edges_of(k).collect();
            for e in edges.iter().take(limit) {
                println!("    {label:<9} {:>10} -> {}", word(e.from), word(e.to));
            }
            if edges.len() > limit {
                println!("    ... (+{} more {label} edges; raise --limit)", edges.len() - limit);
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
            if f.chords.len() > limit {
                println!("    ... (+{} more chords; raise --limit)", f.chords.len() - limit);
            }
        }
    }
    if show_field {
        println!("  per-token (token -> depth | enclosing heads, outer first):");
        let enclosed: Vec<usize> = (0..f.frames.len()).filter(|&i| f.frames[i].depth > 0).collect();
        for &i in enclosed.iter().take(limit) {
            let fr = &f.frames[i];
            let heads: Vec<String> = fr.enclosure.iter().map(|&h| word(h)).collect();
            println!("    {:<14} d={:>2}  [{}]", format!("{:.14}", word(i)), fr.depth, heads.join(" > "));
        }
        if enclosed.len() > limit {
            println!("    ... (+{} more tokens; raise --limit)", enclosed.len() - limit);
        }
    }
    ExitCode::SUCCESS
}

fn run_stress(args: &[String]) -> ExitCode {
    const COMMAND: &str = "stress";
    let mut input: Option<Vec<u8>> = None;
    let mut show_field = false;
    let mut show_peaks = false;
    let mut show_fractures = false;
    let mut limit = usize::MAX;
    let mut cfg = trex::stress::StressConfig::default();
    let mut decls = trex::Declarations::new();
    let mut i = 0;
    let mut paths_only = false;
    while i < args.len() {
        let arg = args[i].as_str();
        if paths_only || !is_flag(arg) {
            match read_input(arg) {
                Ok(b) => input = Some(b),
                Err(e) => {
                    eprintln!("trex: cannot read {arg}: {e}");
                    return ExitCode::FAILURE;
                }
            }
            i += 1;
            continue;
        }
        take_declaration_or_fail!(args, i, decls);
        match arg {
            "--" => paths_only = true,
            "-h" | "--help" => {
                print_axis_help(COMMAND);
                return ExitCode::SUCCESS;
            }
            "--field" => show_field = true,
            "--peaks" => show_peaks = true,
            "--fractures" => show_fractures = true,
            "--peak-min-depth" => {
                i += 1;
                cfg.peak_min_depth = flag_value_or_fail!("stress", args, i, "--peak-min-depth");
            }
            "--fracture-min-depth" => {
                i += 1;
                cfg.fracture_min_depth = flag_value_or_fail!("stress", args, i, "--fracture-min-depth");
            }
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
            flag => return unknown_flag(COMMAND, flag),
        }
        i += 1;
    }
    let Some(input) = input else {
        eprintln!("trex stress: no input given (pass a FILE or --text STRING)");
        return ExitCode::FAILURE;
    };
    use trex::stress;
    let f = stress::analyze_with(&lex_declared(&input, &decls, true), &input, &cfg);
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
        if f.frames.len() > limit {
            println!("    ... (+{} more tokens; raise --limit)", f.frames.len() - limit);
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
    const COMMAND: &str = "magnitude";
    let mut input: Option<Vec<u8>> = None;
    let mut show_field = false;
    let mut show_jumps = false;
    let mut show_energy = false;
    let mut show_outliers = false;
    let mut limit = usize::MAX;
    let mut top = 3usize;
    let mut cfg = trex::magnitude::MagnitudeConfig::default();
    let mut decls = trex::Declarations::new();
    let mut i = 0;
    let mut paths_only = false;
    while i < args.len() {
        let arg = args[i].as_str();
        if paths_only || !is_flag(arg) {
            match read_input(arg) {
                Ok(b) => input = Some(b),
                Err(e) => {
                    eprintln!("trex: cannot read {arg}: {e}");
                    return ExitCode::FAILURE;
                }
            }
            i += 1;
            continue;
        }
        take_declaration_or_fail!(args, i, decls);
        match arg {
            "--" => paths_only = true,
            "-h" | "--help" => {
                print_axis_help(COMMAND);
                return ExitCode::SUCCESS;
            }
            "--field" => show_field = true,
            "--jumps" => show_jumps = true,
            "--energy" => show_energy = true,
            "--outliers" => show_outliers = true,
            "--jump-threshold" => {
                i += 1;
                cfg.jump_threshold = cut_value_or_fail!("magnitude", args, i, "--jump-threshold");
            }
            "--outlier-sigma" => {
                i += 1;
                cfg.outlier_sigma = cut_value_or_fail!("magnitude", args, i, "--outlier-sigma");
            }
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
            flag => return unknown_flag(COMMAND, flag),
        }
        i += 1;
    }
    let Some(input) = input else {
        eprintln!("trex magnitude: no input given (pass a FILE or --text STRING)");
        return ExitCode::FAILURE;
    };
    use trex::magnitude;
    let f = magnitude::analyze_with(&lex_declared(&input, &decls, true), &input, &cfg);
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
        if f.frames.len() > limit {
            println!("    ... (+{} more tokens; raise --limit)", f.frames.len() - limit);
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
        // A ranked list: the header names its length, set by --top, and the
        // total.
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
    const COMMAND: &str = "shape";
    let mut input: Option<Vec<u8>> = None;
    let mut classes = false;
    let mut period = false;
    let mut segment = false;
    let mut json = false;
    let mut orbit_group: Option<trex::orbit::OrbitGroup> = None;
    let mut limit = usize::MAX;
    let mut cfg = trex::shape::ShapeConfig::default();
    let mut decls = trex::Declarations::new();
    let mut i = 0;
    let mut paths_only = false;
    while i < args.len() {
        let arg = args[i].as_str();
        if paths_only || !is_flag(arg) {
            match read_input(arg) {
                Ok(b) => input = Some(b),
                Err(e) => {
                    eprintln!("trex: cannot read {arg}: {e}");
                    return ExitCode::FAILURE;
                }
            }
            i += 1;
            continue;
        }
        take_declaration_or_fail!(args, i, decls);
        match arg {
            "--" => paths_only = true,
            "-h" | "--help" => {
                print_axis_help(COMMAND);
                return ExitCode::SUCCESS;
            }
            "--classes" => classes = true,
            "--period" => period = true,
            "--segment" => segment = true,
            "--json" => json = true,
            "--limit" => {
                i += 1;
                limit = flag_value_or_fail!("shape", args, i, "--limit");
            }
            "--template-strength" => {
                i += 1;
                cfg.template_strength = cut_value_or_fail!("shape", args, i, "--template-strength");
            }
            "--group" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --group needs a group (identity|case|notation|shape|e8|ip|url|time|path|fold|numeric, or a typed relation such as subnet/24)");
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
            flag => return unknown_flag(COMMAND, flag),
        }
        i += 1;
    }
    let Some(input) = input else {
        eprintln!("trex shape: no input given (pass a FILE or --text STRING)");
        return ExitCode::FAILURE;
    };
    use trex::shape;
    let toks = lex_declared(&input, &decls, true);
    let field = match orbit_group {
        Some(g) => shape::analyze_over_with(&toks, &input, g, &cfg),
        None => shape::analyze_with(&toks, &input, &cfg),
    };
    let regions = field.shape_regions();
    let dom = field
        .frames
        .iter()
        .filter(|f| f.period > 0)
        .max_by(|a, b| a.period_strength.total_cmp(&b.period_strength))
        .map(|f| (f.period, f.period_strength));

    if json {
        let (dominant_period, dominant_strength) = dom.unwrap_or((0, 0.0));
        print!(
            "{{\"n_tokens\":{},\"dominant_period\":{dominant_period},\"dominant_strength\":{dominant_strength:.4},\"boundaries\":[",
            field.n_tokens
        );
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
        print!("],\"frames\":[");
        for (k, (f, &(s, e))) in field.frames.iter().zip(&field.spans).enumerate() {
            if k > 0 {
                print!(",");
            }
            print!(
                "{{\"start\":{s},\"end\":{e},\"class\":{},\"period\":{},\"period_strength\":{:.4},\"novelty\":{:.4}}}",
                f.class, f.period, f.period_strength, f.novelty
            );
        }
        println!("]}}");
        return ExitCode::SUCCESS;
    }

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
        println!("  per-token (token -> class | period | strength | novelty):");
        for (k, f) in field.frames.iter().enumerate().take(limit) {
            let (s, e) = field.spans[k];
            let w = String::from_utf8_lossy(&input[s..e]).replace('\n', "\\n");
            println!(
                "    {:<14} cls={:08x} per={:<3} str={:.2} nov={:.2}",
                format!("{w:.14}"),
                f.class,
                f.period,
                f.period_strength,
                f.novelty
            );
        }
        if field.frames.len() > limit {
            println!("    ... (+{} more tokens; raise --limit)", field.frames.len() - limit);
        }
    }
    ExitCode::SUCCESS
}

fn run_spectral(args: &[String]) -> ExitCode {
    const COMMAND: &str = "spectral";
    let mut input: Option<Vec<u8>> = None;
    let mut segment = false;
    let mut bands = false;
    let mut timeline = false;
    let mut classify = false;
    let mut json = false;
    let mut limit = usize::MAX;
    let mut cfg = trex::spectral::SpectralConfig::default();
    let mut i = 0;
    let mut paths_only = false;
    while i < args.len() {
        let arg = args[i].as_str();
        if paths_only || !is_flag(arg) {
            match read_input(arg) {
                Ok(b) => input = Some(b),
                Err(e) => {
                    eprintln!("trex: cannot read {arg}: {e}");
                    return ExitCode::FAILURE;
                }
            }
            i += 1;
            continue;
        }
        match arg {
            "--" => paths_only = true,
            "-h" | "--help" => {
                print_axis_help(COMMAND);
                return ExitCode::SUCCESS;
            }
            "--segment" => segment = true,
            "--bands" => bands = true,
            "--timeline" => timeline = true,
            "--classify" => classify = true,
            "--json" => json = true,
            "--limit" => {
                i += 1;
                limit = flag_value_or_fail!("spectral", args, i, "--limit");
            }
            "--cp-threshold" => {
                i += 1;
                cfg.cp_threshold = cut_value_or_fail!("spectral", args, i, "--cp-threshold");
            }
            "--cp-floor" => {
                i += 1;
                cfg.cp_floor = cut_value_or_fail!("spectral", args, i, "--cp-floor");
            }
            "--cp-min-gap" => {
                i += 1;
                cfg.cp_min_gap = flag_value_or_fail!("spectral", args, i, "--cp-min-gap");
            }
            "--text" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --text needs a value");
                    return ExitCode::FAILURE;
                };
                input = Some(v.clone().into_bytes());
            }
            flag => return unknown_flag(COMMAND, flag),
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
    // grammar-free region map the construct / disasm layers read, each region's
    // texture read from a fresh pass over its own bytes so one region's bytes
    // do not change another's label.
    if classify {
        // Fused region classification: spectral texture x shape period. A
        // tabular block reads as one `table` region (the shape axis), where the
        // byte-period alone reads only "data".
        let regions = trex::shape::classified_regions_with(&input, &cfg);
        println!(
            "trex spectral (region classification: shape x spectral): {} bytes, {} regions",
            input.len(),
            regions.len()
        );
        for (s, e, kind) in &regions {
            // The preview starts at the start of the character holding the
            // region's first byte, as the other surfaces place a point inside
            // a character, and runs for up to 28 bytes, ending before a
            // character the cut would split.
            let mut start = *s;
            while start > 0 && *s - start < 3 && input[start] & 0xC0 == 0x80 {
                start -= 1;
            }
            let mut end = (start + 28).min(*e);
            while end > start && end < input.len() && input[end] & 0xC0 == 0x80 {
                end -= 1;
            }
            let preview = String::from_utf8_lossy(&input[start..end]).replace(['\n', '\t'], " ");
            let label = match kind {
                trex::shape::RegionKind::Table(p) => format!("table(p{p})"),
                other => format!("{other:?}").to_lowercase(),
            };
            println!("    [{s:>8}..{e:<8}] {label:<10} {preview}");
        }
        return ExitCode::SUCCESS;
    }
    let field = spectral::analyze_with(&input, &cfg);

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
    if timeline || (!segment && !bands) {
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
        if field.frames.len() > limit {
            println!("    ... (+{} more frames; raise --limit)", field.frames.len() - limit);
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

/// The top-level `compress` command in a build without the `compress` feature:
/// `--help` prints the usage and how to build the command, and anything else
/// fails with a message saying so.
#[cfg(not(feature = "compress"))]
fn run_compress(args: &[String]) -> ExitCode {
    if opens_with_help(args) {
        println!("usage: trex compress (FILE | --text STRING) [--compare] [--no-baked] [--prior-cache DIR] [--max-bytes N] [--chunks N] [--gpu | --hybrid] [--second-mixer]");
        println!("  {}", compress_absent("trex compress"));
        return ExitCode::SUCCESS;
    }
    eprintln!("{}", compress_absent("trex compress"));
    ExitCode::FAILURE
}

/// The top-level `compress` command: run the coder and report the compression
/// it achieves (size, ratio, bits/byte), with the default logistic-mix + orbit
/// coder using the baked prior. `--compare` prints the full coder table.
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
    let mut paths_only = false;
    while i < args.len() {
        match args[i].as_str() {
            path if paths_only || !is_flag(path) => {
                if file.is_some() {
                    eprintln!("trex compress: unexpected argument {path}");
                    return ExitCode::FAILURE;
                }
                file = Some(path.to_string());
            }
            "--" => paths_only = true,
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
                println!(
                    "usage: trex compress (FILE | --text STRING) [--compare] [--no-baked] [--prior-cache DIR] [--max-bytes N] [--chunks N] [--gpu | --hybrid] [--second-mixer]"
                );
                println!("  the achieved compression of the default coder: the logistic mix with the orbit");
                println!("  context, over the baked prior");
                println!("  --compare              every coder's bits per byte and speed");
                println!("  --no-baked             code with no prior");
                println!("  --prior-cache DIR      map the baked prior from DIR, writing it there on the first");
                println!("                         run, instead of decoding it every run");
                println!("  --max-bytes N          code the first N bytes");
                println!("  --chunks N             code N slices across the cores, 0 for every core (1 by default)");
                println!("  --gpu, --hybrid        code on the device, or on the device and the CPU at once");
                println!("  --second-mixer         a second mixer beside the shipped one, its weights chosen by");
                println!("                         the stream's rhythm (the power and the group delay of a");
                println!("                         resonator bank over the token kinds), the side information a");
                println!("                         decoder would need counted in; a code length 0.06-4.5% shorter");
                println!("                         on each of seven measured files, for up to 24% more time");
                println!("  a FILE that begins with a dash goes after --");
                return ExitCode::SUCCESS;
            }
            flag => return unknown_flag("compress", flag),
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
    const COMMAND: &str = "seam";
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
    let mut cut_threshold = trex::seam::SeamConfig::default().cut_threshold;
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
    let mut paths_only = false;
    while i < args.len() {
        let arg = args[i].as_str();
        if paths_only || !is_flag(arg) {
            match read_input(arg) {
                Ok(b) => input = Some(b),
                Err(e) => {
                    eprintln!("trex: cannot read {arg}: {e}");
                    return ExitCode::FAILURE;
                }
            }
            i += 1;
            continue;
        }
        match arg {
            "--" => paths_only = true,
            "-h" | "--help" => {
                print_axis_help(COMMAND);
                return ExitCode::SUCCESS;
            }
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
            "--grain-separation" => grain = true,
            "--cut-threshold" => {
                i += 1;
                cut_threshold = cut_value_or_fail!("seam", args, i, "--cut-threshold");
            }
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
            flag => return unknown_flag(COMMAND, flag),
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
        // Vigilance before top-k, so the row budget is spent on rows that
        // predict something their backoff parent does not. top-k ranks by total count,
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
            "trex seam --grain-separation: {} instructions, {} functions, {} blocks (synthetic)",
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
        // With a file, the corpus is that file's real words with the spaces removed, the fairer
        // test, because the synthetic vocabulary below is 15 words that recur constantly and
        // favors any recurrence rule. Without one, the synthetic corpus is used, and its seed is a flag
        // rather than a constant so a comparison can be run over a spread rather than one draw.
        let (bytes, truth) = match &input {
            Some(text) => seam::despaced_text(text, max_bytes.unwrap_or(1 << 17)),
            None => seam::spaceless_corpus(VOCAB, words, corpus_seed),
        };
        let cfg = SeamConfig { order, cut_threshold, passes: passes_override.unwrap_or(0), ..SeamConfig::default() };
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
    let cfg = SeamConfig { order, cut_threshold, passes: passes_override.unwrap_or(0), ..SeamConfig::default() };
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
        print!("],\"boundary\":[");
        for (k, v) in field.boundary.iter().enumerate() {
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
        println!("  per-byte branching entropy, and the strength of a cut before the byte:");
        for (t, &b) in input.iter().enumerate().take(limit) {
            let c = if b.is_ascii_graphic() { b as char } else { '.' };
            println!(
                "    {t:>6} {c}  fwd={:.2} bwd={:.2} str={:.2}",
                field.fwd_entropy[t], field.bwd_entropy[t], field.boundary[t]
            );
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
    const COMMAND: &str = "orbit";
    use trex::orbit::{self, OrbitGroup};
    let mut input: Option<Vec<u8>> = None;
    let mut group = OrbitGroup::Shape;
    let mut collapse = false;
    let mut boundary = false;
    let mut query: Option<Vec<u8>> = None;
    let mut limit = usize::MAX;
    let mut decls = trex::Declarations::new();
    let mut i = 0;
    let mut paths_only = false;
    while i < args.len() {
        let arg = args[i].as_str();
        if paths_only || !is_flag(arg) {
            match read_input(arg) {
                Ok(b) => input = Some(b),
                Err(e) => {
                    eprintln!("trex: cannot read {arg}: {e}");
                    return ExitCode::FAILURE;
                }
            }
            i += 1;
            continue;
        }
        take_declaration_or_fail!(args, i, decls);
        match arg {
            "--" => paths_only = true,
            "-h" | "--help" => {
                print_axis_help(COMMAND);
                return ExitCode::SUCCESS;
            }
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
            "--same-as" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --same-as needs a text");
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
            flag => return unknown_flag(COMMAND, flag),
        }
        i += 1;
    }
    let Some(input) = input else {
        eprintln!("trex orbit: no input given (pass a FILE or --text STRING)");
        return ExitCode::FAILURE;
    };

    // Equivariant match: every span in the query's orbit (matches up to the group).
    let orbit_tokens = || orbit::tokenize_tokens(&lex_declared(&input, &decls, false), &input, group);
    if let Some(q) = query {
        let key = orbit::canonical(&q, group);
        let hits: Vec<orbit::OrbitToken> = orbit_tokens().into_iter().filter(|t| t.orbit == key).collect();
        println!(
            "trex orbit --same-as {:?} (group {}): {} spans in the same orbit",
            String::from_utf8_lossy(&q),
            group.label(),
            hits.len()
        );
        for t in hits.iter().take(limit) {
            println!("    [{:>6}..{:<6}] {:?}", t.start, t.end, t.raw);
        }
        if hits.len() > limit {
            println!("    ... (+{} more spans; raise --limit)", hits.len() - limit);
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
        if segs.len() > limit {
            println!("    ... (+{} more segments; raise --limit)", segs.len() - limit);
        }
        return ExitCode::SUCCESS;
    }

    // Orbit-collapse: distinct raw forms folding onto each orbit representative.
    if collapse {
        let toks = orbit_tokens();
        let (raw, orb) = orbit::collapse_stats_of(&toks);
        let table = orbit::collapse_of(toks);
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
        if table.len() > limit {
            println!("    ... (+{} more orbits; raise --limit)", table.len() - limit);
        }
        return ExitCode::SUCCESS;
    }

    // Default: list each token with its orbit representative.
    let toks = orbit_tokens();
    println!("trex orbit (group {}): {} tokens", group.label(), toks.len());
    for t in toks.iter().take(limit) {
        println!("    [{:>6}..{:<6}] {:?} -> {:?}", t.start, t.end, t.raw, t.orbit);
    }
    if toks.len() > limit {
        println!("    ... (+{} more tokens; raise --limit)", toks.len() - limit);
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
    let mut paths_only = false;
    while i < args.len() {
        match args[i].as_str() {
            path if paths_only || !is_flag(path) => match read_input(path) {
                Ok(b) => corpus = Some(b),
                Err(e) => {
                    eprintln!("trex: cannot read {path}: {e}");
                    return ExitCode::FAILURE;
                }
            },
            "--" => paths_only = true,
            "-h" | "--help" => {
                println!("usage: trex prefilter (FILE | --text STRING) [--literal L]... [--filter bloom|cuckoo|xor] [--verify]");
                println!("  which literals might occur in the input, by a filter over its n-grams that never");
                println!("  answers no for a literal that occurs");
                println!("  --literal L            a literal to test, repeatable");
                println!("  --filter NAME          bloom (the default), cuckoo or xor");
                println!("  --verify               check every literal that occurs reads as a possible match in all three");
                println!("  a FILE that begins with a dash goes after --");
                return ExitCode::SUCCESS;
            }
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
            flag => return unknown_flag("prefilter", flag),
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
/// rejects, which is the scanning the prefilter saves.
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
