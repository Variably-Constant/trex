//! `trex tokens` and `trex escape`: the tokens trex reads an input as, and a
//! text written as the pattern that matches it literally.

use std::process::ExitCode;

use trex::token::TokenKind;

/// `trex tokens (FILE | --text S) [--whitespace] [--lib F] [--shape D]
/// [--shape-after D] [--kind D] [--let D] [--declare L] [--json] [--binary]`:
/// each token of the input as `[start..end] kind "text"`, in byte offsets as
/// a scan prints a span, or with `--json` as an array of objects carrying the
/// value each token parses to. Whitespace is left out unless `--whitespace`
/// asks for it, since a pattern never matches it, and a file holding a NUL
/// byte is refused unless `--binary` asks for it, as a scan refuses one.
pub(crate) fn run_tokens(args: &[String]) -> ExitCode {
    let mut text: Option<Vec<u8>> = None;
    let mut file: Option<String> = None;
    let mut whitespace = false;
    let mut json = false;
    let mut binary = false;
    let mut decls = trex::Declarations::new();
    let mut i = 0;
    let mut paths_only = false;
    while i < args.len() {
        match args[i].as_str() {
            given if paths_only || !crate::is_flag(given) => {
                if let Some(first) = &file {
                    eprintln!("trex tokens: lists one FILE, and {first} and {given} were given");
                    return ExitCode::FAILURE;
                }
                file = Some(given.to_string());
            }
            "--" => paths_only = true,
            "-h" | "--help" => {
                println!("{TOKENS_USAGE}");
                println!("  each token of the input as [start..end] kind \"text\"");
                println!("  --whitespace           the whitespace tokens too");
                println!("  --json                 each token as an object with its parsed value");
                println!("  --binary               lex a file that holds a NUL byte");
                println!("  --lib, --shape, --shape-after, --kind, --let, --declare");
                println!("                         lex under declarations, read in order, as a scan does");
                println!("  a FILE that begins with a dash goes after --");
                return ExitCode::SUCCESS;
            }
            "--text" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    eprintln!("trex: --text needs a value");
                    return ExitCode::FAILURE;
                };
                text = Some(v.clone().into_bytes());
            }
            "--shape" | "--shape-after" | "--kind" | "--let" | "--declare" => {
                i += 1;
                if let Err(e) = crate::cli_files::declare_flag(&mut decls, &args[i - 1], args.get(i)) {
                    eprintln!("trex: {e}");
                    return ExitCode::FAILURE;
                }
            }
            "--lib" => {
                i += 1;
                let Some(path) = args.get(i) else {
                    eprintln!("trex: --lib needs a pattern file, or a directory of .trex files");
                    return ExitCode::FAILURE;
                };
                if let Err(e) = crate::cli_files::declare_file(&mut decls, path) {
                    eprintln!("trex: {e}");
                    return ExitCode::FAILURE;
                }
            }
            "--whitespace" => whitespace = true,
            "--json" => json = true,
            "--binary" => binary = true,
            flag => return crate::unknown_flag("tokens", flag),
        }
        i += 1;
    }
    let shapes = decls.set();
    let input = match (text, file) {
        (Some(bytes), None) => bytes,
        (None, Some(path)) => {
            let raw = match std::fs::read(&path) {
                Ok(raw) => raw,
                Err(e) => {
                    eprintln!("trex: cannot read {path}: {e}");
                    return ExitCode::FAILURE;
                }
            };
            if !binary && trex::files::is_binary(&raw) {
                eprintln!("trex: {path} holds a NUL byte and is binary; --binary lexes it");
                return ExitCode::FAILURE;
            }
            trex::encoding::decode(raw)
        }
        (Some(_), Some(_)) => {
            eprintln!("trex tokens: give FILE or --text STRING, not both");
            return ExitCode::FAILURE;
        }
        (None, None) => {
            eprintln!("{TOKENS_USAGE}");
            return ExitCode::FAILURE;
        }
    };
    let toks = if shapes.is_empty() {
        trex::lexer::lex(&input)
    } else {
        trex::lexer::lex_with_shapes(&input, &trex::lexer::blob_runs(&input), shapes, 0)
    };
    let clock = trex::Clock::current();
    let mut objects: Vec<String> = Vec::new();
    for t in &toks {
        if !whitespace && !t.is_significant() {
            continue;
        }
        let (start, end) = (t.start(), t.end());
        let piece = String::from_utf8_lossy(&input[start..end]);
        let kind = shapes.kind_name(t.kind);
        if !json {
            crate::out::line(&format!("[{start}..{end}] {kind} {piece:?}"));
            continue;
        }
        // A declared shape or kind carries no value of its own to parse.
        let value = match t.kind {
            TokenKind::Custom(_) => None,
            known => trex::typed::value_of(known, &piece),
        };
        let value = match value {
            Some(v) => v.json(t.kind, trex::typed::ValueStyle::new(), clock),
            None => "null".to_string(),
        };
        objects.push(format!(
            "{{\"kind\":\"{}\",\"start\":{start},\"end\":{end},\"text\":\"{}\",\"value\":{value}}}",
            crate::json_escape(&kind),
            crate::json_escape(&piece)
        ));
    }
    if json {
        crate::out::line(&format!("[{}]", objects.join(",")));
    }
    ExitCode::SUCCESS
}

/// The usage `trex tokens` prints.
const TOKENS_USAGE: &str = "usage: trex tokens (FILE | --text STRING) [--whitespace] [--lib FILE|DIR] [--shape|--shape-after|--kind|--let DECL] [--declare LINE] [--json] [--binary]";

/// `trex escape TEXT`: the pattern that matches `TEXT` literally. A text that
/// begins with a dash goes after `--`.
pub(crate) fn run_escape(args: &[String]) -> ExitCode {
    let args = match args {
        [flag] if flag == "-h" || flag == "--help" => {
            println!("usage: trex escape TEXT");
            println!("  the pattern that matches TEXT exactly, each of its tokens a quoted literal");
            println!("  a TEXT that begins with a dash goes after --");
            return ExitCode::SUCCESS;
        }
        [dashes, rest @ ..] if dashes == "--" => rest,
        [flag, ..] if crate::is_flag(flag) => return crate::unknown_flag("escape", flag),
        _ => args,
    };
    match args {
        [text] => {
            crate::out::line(&trex::escape(text));
            ExitCode::SUCCESS
        }
        [] => {
            eprintln!("usage: trex escape TEXT");
            ExitCode::FAILURE
        }
        [_, rest @ ..] => {
            eprintln!(
                "trex escape: takes one TEXT, and {} more were given; quote a text holding spaces",
                rest.len()
            );
            ExitCode::FAILURE
        }
    }
}
