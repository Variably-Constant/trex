//! Metamorphic streaming test: feeding the input in chunks of any size
//! through the public streaming API must yield exactly the matches a
//! whole-input scan produces. This is the cross-chunk equivalence
//! contract, exercised over many patterns and chunk sizes.

use trex::{StreamScanner, parse, scan, scan_chunked};

/// Feed `input` in fixed-size chunks and assert equivalence to the
/// whole-input scan.
fn equiv(pattern_src: &str, input: &str, chunk: usize) {
    let pat = parse(pattern_src).expect("pattern parses");
    let whole = scan(&pat, input.as_bytes());
    let bytes = input.as_bytes();
    let chunks: Vec<&[u8]> = bytes.chunks(chunk.max(1)).collect();
    let streamed = scan_chunked(&pat, chunks);
    assert_eq!(streamed, whole, "pattern {pattern_src:?} chunk={chunk}");
}

#[test]
fn chunked_equals_whole_over_a_pattern_and_size_matrix() {
    let cases: &[(&str, &str)] = &[
        ("\\N \\W", "weight 12 kg\nlen 5 m\nmass 9 g\nrate 3 hz\n"),
        ("\\W:x =x", "the the cat\ndog dog ran\nsame same now\nfoo bar baz\n"),
        ("<\\W:t>.*</=t>", "<a>x</a>\n<b>yy</b>\n<c>zzz</c>\n<d>q</d>\n"),
        (". ~\"END\"", "begin here END\nmore lines END now\nlast line END\n"),
        ("\\W\\B(.*)", "call f(g(x))\nrun h(k(y))\nwrap a(b(c(d)))\ntail\n"),
        (".*", "anything at all here\nand more on this line\nthen done\n"),
        ("@3 \"ERROR\"", "a, b, ERROR\nx, y, OK\np, q, ERROR\n"),
        ("\\I ~\"down\"", "10.0.0.1 is down\n192.168.1.1 is up\nfe80::1 down\n"),
        ("\\T", "2026-06-16 12:30:45\nlog at 09:15\nstamp 2026-01-01T00:00:00\n"),
        // A bounded pattern whose match crosses a line: the tokens that can
        // still begin one are kept past every cut.
        ("\\N \\W \"=\"", "a = 1\nb c =\nx 2 y\n9 z =\n"),
        ("\\W:w \"=\" \\N", "a = 1\nb = 2\nc d\n= 3\n"),
    ];
    for (pat, input) in cases {
        for chunk in [1usize, 2, 3, 4, 5, 8, 16, 32, 100, 10_000] {
            equiv(pat, input, chunk);
        }
    }
}

#[test]
fn a_line_anchored_pattern_streams_as_the_whole_input_scans() {
    let input = "GET /a 200\nGET /b 204 extra\nnote GET /c 201\nGET /d 500\r\n\nGET /e 301\n  GET /f 302\nGET /g";
    let cases = [
        "^ \"GET\" \"/\" \\W (\\N):status",
        "\"GET\" \"/\" \\W $ (\\N):status",
        "^ \"GET\" \"/\" \\W $ (\\N):status",
        "^ \"GET\" \"/\" \\W (\\N):status ~<($ .)",
        "^ (\"GET\" \"/\" \\W .*? $ .):line ~<($ .)",
        "^ \\W .+? ~<(\\N) | ^ \"note\" .*? $ .",
        // A lookbehind that can reach before its match stays deferred, and
        // agrees either way.
        "~<(\"GET\" \"/\" \\W) \\N",
        "~<(\\N) \"GET\"",
    ];
    for pat in cases {
        for chunk in [1usize, 2, 3, 5, 7, 11, 16, 64, 10_000] {
            equiv(pat, input, chunk);
        }
    }
}

#[test]
fn a_line_anchored_pattern_commits_each_line_as_it_ends() {
    let line = "GET /index.html 200\n";
    for src in ["^ \"GET\" \\L $ (\\N):status", "^ \"GET\" \\L (\\N):status ~<($ .)"] {
        let pat = parse(src).expect("pattern parses");
        let mut s = StreamScanner::new(pat);
        assert!(s.commits_early(), "{src} is held to the stream's end");
        let mut found = 0;
        for _ in 0..2_000 {
            s.push(line.as_bytes());
            found += s.drain_committed().len();
        }
        let peak = s.peak_retained();
        found += s.finish().len();
        assert_eq!(found, 2_000, "{src}");
        assert!(peak <= 4 * line.len(), "{src} retained {peak} bytes at once");
    }
    let lookbehind = parse("~<(\\N) \"GET\"").expect("pattern parses");
    assert!(!StreamScanner::new(lookbehind).commits_early(), "a lookbehind before its match reads a line the stream may have dropped");
}

#[test]
fn incremental_push_then_finish_matches_whole() {
    // Drive the StreamScanner by hand, pushing uneven chunks, and check
    // the final result equals a whole-input scan.
    let input = b"the the dog\nrun run now\nsame same here\nlast last line\n";
    let pat = parse("\\W:x =x").expect("pattern parses");
    let whole = scan(&pat, input);

    let mut s = StreamScanner::new(pat.clone());
    // Deliberately uneven, boundary-crossing pushes.
    for piece in [&input[..5], &input[5..6], &input[6..23], &input[23..24], &input[24..]] {
        s.push(piece);
    }
    let streamed = s.finish();
    assert_eq!(streamed, whole);
}

#[test]
fn an_unbounded_match_reaching_back_over_a_cut_is_not_dropped() {
    // The pattern's match has no bounded length, so no count of tokens says
    // how far back a match completed by a later chunk may begin. Here the
    // first chunk holds no match at all, and a scanner that trimmed to its
    // last line boundary on that basis would drop the words the match
    // needs: `alpha` and `beta` belong to the match `gamma END` completes.
    let input = b"alpha\nbeta\ngamma END\n";
    let pat = parse("\\W+ \"END\"").expect("pattern parses");
    let whole = scan(&pat, input);
    assert_eq!(whole.len(), 1, "the whole-input scan holds one match");
    assert_eq!(whole[0].start(), 0, "which reaches back to the first word");
    for cut in 1..input.len() {
        let mut s = StreamScanner::new(pat.clone());
        s.push(&input[..cut]);
        s.push(&input[cut..]);
        assert_eq!(s.finish(), whole, "split at {cut}");
    }
}

#[test]
fn an_unbounded_pattern_releases_the_buffer_once_no_attempt_reaches_back() {
    // The fix must not turn every unbounded pattern into one that retains
    // the whole stream: a line that cannot begin a match is still dropped,
    // so the retained window stays bounded over a long input.
    let mut input = String::new();
    for i in 0..400 {
        input.push_str(&format!("plain line {i} with no marker\n"));
    }
    let pat = parse("\\W+ \"END\"").expect("pattern parses");
    let mut s = StreamScanner::new(pat);
    for chunk in input.as_bytes().chunks(64) {
        s.push(chunk);
    }
    assert!(
        s.retained() < input.len() / 4,
        "the window grew to {} of {} bytes",
        s.retained(),
        input.len()
    );
}

#[test]
fn large_input_streams_equivalently() {
    // A larger, multi-line input crossing many internal boundaries.
    let mut input = String::new();
    for i in 0..2000 {
        input.push_str(&format!("row {i} value {} tag t{}\n", i * 3, i % 7));
    }
    let pat = parse("\\W \\N").expect("pattern parses");
    let whole = scan(&pat, input.as_bytes());
    for chunk in [1usize, 64, 997, 65_536] {
        let chunks: Vec<&[u8]> = input.as_bytes().chunks(chunk).collect();
        assert_eq!(scan_chunked(&pat, chunks), whole, "large input chunk={chunk}");
    }
}
