//! The complete pattern-language catalog: every construct trex can match,
//! each shown against a real input with its matches printed - and asserted,
//! so this example is also an end-to-end check that every construct works.
//!
//! Run: `cargo run --example patterns`
//!
//! Sections mirror the pattern-syntax reference: token atoms, named atoms,
//! byte classes, literals and the byte-regex, grouping and quantifiers,
//! alternation, balanced groups, binding and back-references, content guards,
//! field anchors, lenses, silhouettes, axis predicates, statistical anchors,
//! and rewrite templates with typed sub-field accessors.

use trex::{Template, captures, parse, rewrite, scan};

/// Scan `input` with `pattern`, print every match with the registers it
/// bound, and require exactly `expect` matches - a demo that silently
/// stopped matching is a bug.
fn demo(title: &str, pattern: &str, input: &str, expect: usize) {
    let pat = parse(pattern).unwrap_or_else(|e| panic!("{title}: pattern {pattern:?}: {e:?}"));
    let spans = scan(&pat, input.as_bytes());
    let matches = captures(&pat, input.as_bytes(), &spans);
    println!("  {title}");
    println!("    pattern  {pattern}");
    println!("    input    {input:?}");
    for m in &matches {
        let text = String::from_utf8_lossy(&input.as_bytes()[m.start..m.end]);
        if m.captures().is_empty() {
            println!("    match    [{}..{}] {text:?}", m.start, m.end);
        } else {
            let caps: Vec<String> = m
                .names()
                .iter()
                .zip(m.captures())
                .map(|(k, s)| {
                    format!("{k}={:?}", String::from_utf8_lossy(&input.as_bytes()[s.range()]))
                })
                .collect();
            println!("    match    [{}..{}] {text:?}  {}", m.start, m.end, caps.join(" "));
        }
    }
    assert_eq!(
        matches.len(),
        expect,
        "{title}: expected {expect} match(es), got {}",
        matches.len()
    );
    println!();
}

/// Like [`demo`], but for field-driven constructs where every qualifying
/// token matches (spectral predicates, seam cuts): assert at least `min`
/// matches and print the actual count.
fn demo_min(title: &str, pattern: &str, input: &str, min: usize) {
    let pat = parse(pattern).unwrap_or_else(|e| panic!("{title}: pattern {pattern:?}: {e:?}"));
    let matches = scan(&pat, input.as_bytes());
    println!("  {title}");
    println!("    pattern  {pattern}");
    println!("    input    {input:?}");
    for m in matches.iter().take(3) {
        let text = String::from_utf8_lossy(&input.as_bytes()[m.range()]);
        println!("    match    [{}..{}] {text:?}", m.start, m.end);
    }
    if matches.len() > 3 {
        println!("    ... (+{} more; {} total)", matches.len() - 3, matches.len());
    }
    assert!(matches.len() >= min, "{title}: expected >= {min}, got {}", matches.len());
    println!();
}

/// Rewrite `input` with `pattern` + `template` and print the result.
fn demo_rewrite(title: &str, pattern: &str, template: &str, input: &str, expect_out: &str) {
    let pat = parse(pattern).unwrap_or_else(|e| panic!("{title}: pattern: {e:?}"));
    let tpl = Template::parse(template, &pat.capture_names())
        .unwrap_or_else(|e| panic!("{title}: template: {e:?}"));
    let out = rewrite(&pat, &tpl, input.as_bytes());
    let out = String::from_utf8_lossy(&out);
    println!("  {title}");
    println!("    pattern  {pattern}");
    println!("    template {template}");
    println!("    input    {input:?}");
    println!("    output   {out:?}");
    assert_eq!(out, expect_out, "{title}");
    println!();
}

#[allow(clippy::too_many_lines)]
fn main() {
    println!("== 1. Token atoms: one typed token per escape ==\n");
    demo("number", r"\N", "width 42 height", 1);
    demo("word / identifier", r"\W", "... some_ident ...", 1);
    // A unicode word is one token - a CJK run never shatters into byte shards.
    demo("word (CJK, one token)", r"\W", "\u{4E2D}\u{6587}\u{5206}\u{8BCD}", 1);
    demo("quoted string", r#"\Q"#, r#"say "hello world" now"#, 1);
    demo("IPv4", r"\I", "from 192.168.5.9 accepted", 1);
    demo("IPv6 (same atom)", r"\I", "addr 2001:db8::8a2e:370:7334 up", 1);
    demo("URL", r"\U", "get https://example.com:8080/a?q=1 done", 1);
    demo("email", r"\E", "mail bob@example.com now", 1);
    // A date and a clock joined by one space are one Timestamp token; a date
    // on its own is one too.
    demo("timestamp (a date and its clock are one token)", r"\T", "at 2026-07-06 14:31:07 ok", 1);
    demo("timestamp (a date alone)", r"\T", "since 2026-07-06 and 14:31", 2);
    demo("punctuation", r"\P", "a , b", 1);
    // Matches anchor at significant tokens, so \S cannot START a pattern; it
    // matches whitespace explicitly between atoms.
    demo("whitespace (explicit, between atoms)", r"\W \S \W", "a b", 1);
    demo("semantic version", r"\V", "release v1.2.3-rc1 out", 1);
    demo("UUID", r"\{uuid}", "id 550e8400-e29b-41d4-a716-446655440000 ok", 1);
    demo("MAC address", r"\{mac}", "nic 01:23:45:67:89:ab up", 1);
    demo("hex color", r"\H", "background #ff8800 border", 1);
    demo("CIDR block", r"\C", "route 10.0.0.0/8 via", 1);
    demo("byte size", r"\Z", "quota 1.5GiB left", 1);
    demo("percentage", r"\%", "cpu 93% busy", 1);
    demo("money", r"\$", "cost $1,234.56 total", 1);
    demo("hash digest", r"\D", "sha 5ba93c9db0cff93f52b521d7420e43f6eda2784f ok", 1);
    demo("duration", r"\R", "took 1500ms wall", 1);
    demo("filesystem path", r"\L", "open /usr/bin/trex now", 1);
    demo("any one token", r"\N . \N", "1 x 2", 1);

    println!("== 2. Named atoms: kinds past the single letters ==\n");
    demo("JSON Web Token", r"\{jwt}", "auth eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjMifQ.SflKxwRJ ok", 1);
    demo("credit card (Luhn-valid)", r"\{creditcard}", "pay 4111 1111 1111 1111 now", 1);
    demo("Luhn-invalid digits are NOT a card", r"\{creditcard}", "id 1234 5678 1234 5678 x", 0);
    demo("base64 blob", r"\{base64}", "blob aGVsbG8gd29ybGQhIQ== done", 1);
    demo("geo coordinate", r"\{geo}", "pin 37.7749,-122.4194 here", 1);
    demo("a row of columns is NOT a coordinate", r"\{geo}", "row 1.2500,2.5000,3.7500 end", 0);
    demo("phone (+ prefixed)", r"\{phone}", "call +1 555-123-4567 now", 1);
    demo("phone (North American, hyphenated)", r"\{phone}", "call 212-555-1234 now", 1);
    demo("a signed number and figures is NOT a phone", r"\{phone}", "delta +5 10 20 30 40", 0);
    demo("letter atom by full name", r"\{ip}", "from 10.0.0.7 ok", 1);

    println!("== 3. Byte classes: lowercase = the byte grain ==\n");
    demo("all-digit token", r"\d", "abc 123 def", 1);
    demo("word-byte token", r"\w", "... under_score7 ...", 1);
    // Like \S, a byte-class \s cannot START a pattern (matches anchor at
    // significant tokens); it constrains the whitespace between atoms.
    demo("whitespace bytes (between atoms)", r"\W \s \W", "a  b", 1);
    demo("all-hex token", r"\h", "hash deadbeef word", 1);
    demo("all-letter token", r"\a", "abc d3f 42", 1);
    demo("all-uppercase token", r"\u", "ERROR warn INFO", 2);
    demo("all-lowercase token", r"\l", "ERROR warn INFO", 1);

    println!("== 4. Literals, byte-regex, grouping ==\n");
    demo("literal token", r#""ERROR""#, "info ERROR info", 1);
    demo("byte-regex whole-anchored to one token", r"`v[0-9]+`", "rel v2 and v17 but rev2", 2);
    demo("grouping with parens", r"(\W \N)+", "a 1 b 2 c 3", 1);

    println!("== 5. Quantifiers ==\n");
    demo("star (longest)", r"\W \N*", "tag 1 2 3", 1);
    demo("plus", r"\N+", "1 2 3", 1);
    demo("optional", r"\W \N? \W", "a b", 1);
    demo("exact count", r"\N{3}", "9 8 7", 1);
    demo("range count", r"\N{2,3}", "9 8", 1);

    println!("== 6. Alternation (ordered choice) ==\n");
    demo("kind alternation", r"\I | \E", "bob@x.com and 10.0.0.1", 2);

    println!("== 7. Balanced groups: nestable brackets as one primitive ==\n");
    demo("parens with interior", r"\W \B(.*)", "call f(g(x), y) end", 1);
    demo("square brackets", r"\B[.*]", "list [1, [2, 3]] end", 1);
    demo("braces", r"\B{.*}", "block { a { b } } end", 1);

    println!("== 8. Binding and back-references ==\n");
    demo("bind + exact backref", r"\W:x =x", "the the cat", 1);
    demo("matched tag pair", r"<\W:t>.*</=t>", "<div>hi</div>", 1);
    demo("scoped bind inside a group", r"\B(\W::v =v)", "(cat cat) (dog cow)", 1);
    demo("fuzzy backref: same shape", r"\W:x =shape x", "cat dog", 1);
    demo("fuzzy backref: same up to case", r"\W:x =case x", "Foo foo", 1);
    // The notation orbit folds math notation: the Greek glyph and its name
    // are one orbit, so the glyph back-references the word.
    demo("fuzzy backref: same up to notation", r"\W:x =notation x", "theta \u{03B8}", 1);

    println!("== 9. Content guards: the forward window must (not) contain ==\n");
    // The date and its clock are one timestamp token, and it sees CRITICAL in
    // its forward window.
    demo("require a literal ahead", r#"~"CRITICAL" \T"#, "2026-07-06 10:00:00 CRITICAL disk", 1);
    demo("forbid a literal ahead", r#"!~"DEBUG" \T"#, "2026-07-06 10:00:00 CRITICAL disk", 1);
    demo("the same forbid with DEBUG present matches nothing", r#"!~"DEBUG" \T"#, "2026-07-06 10:00:00 DEBUG disk", 0);

    println!("== 10. Field anchor: the k-th comma-delimited field ==\n");
    demo("number in the 2nd comma field (1-based)", r"@2 \N", "alice, 42, reader", 1);

    println!("== 10b. Position anchors: where a token is ==\n");
    // The `@` anchors describe what a token is. These describe where it
    // is, which is what separates a token being used from the same
    // token mentioned later in the line.
    demo(
        "^ : leads its line (or the input)",
        r#"^ "cat""#,
        "cat file\nrun cat\ncat again",
        2,
    );
    demo("^ : indentation still leads", r#"^ "cat""#, "\t cat x", 1);
    demo(
        "$ : ends its line",
        r#"$ "cat""#,
        "run cat\ncat file\nx cat",
        2,
    );
    demo(
        "^ $ : alone on its line",
        r#"^ $ "cat""#,
        "x\ncat\ny",
        1,
    );
    // Composed with alternation the anchors give a position class, with
    // no host syntax baked into the language.
    demo(
        "position class: wherever a command may begin",
        r#"(^ | "|" | ";") "cat""#,
        "cat a\necho the cat sat\nls | cat b",
        2,
    );

    println!("== 11. Lenses: convergent structural shapes ==\n");
    demo("@call", r"@call", "sum = add(a, b)", 1);
    demo("@block", r"@block", "fn f() { body }", 1);
    demo("@nesting (any bracket kind)", r"@nesting", "pair (x)", 1);
    demo("@string", r"@string", r#"say "hi" now"#, 1);
    demo("@number", r"@number", "n = 7", 1);
    demo("@ident", r"@ident", "7 = x", 1);
    demo("@assignment", r"@assignment", "count = 5 more", 1);
    demo("@kv", r"@kv", "port: 8080", 1);
    demo("@flag", r"@flag", "run --verbose now", 1);
    demo("@list", r"@list", "a, b, c end", 1);
    demo("@range", r"@range", "bytes 10..64 read", 1);

    println!("== 12. Silhouette: match by class shape ==\n");
    demo("word ( number , number )", r##"#"W(N,N)""##, "point(3, 4) and word(x, y)", 1);

    println!("== 13. Axis predicates: query a property axis per token ==\n");
    demo("magnitude above 10^6", r"\M{>6}", "retries = 3 ; max_bytes = 5000000000", 1);
    demo("number atom + magnitude", r"\N{mag>3}", "2 20 20000", 1);
    demo_min("high-entropy token", r"\F{entropy>0.6}", "the the the Kx9#qZ!m2@vB8&wQ4 the", 1);
    demo_min("low-entropy token", r"\F{entropy<0.3}", "aaaaaaaaaaaaaaaa aaaaaaaaaaaaaaaa", 1);
    demo_min(
        "texture: prose region (every token in it matches)",
        r"\F{texture:prose}",
        "the quick brown fox jumps over the lazy dog again and again",
        1,
    );
    // The period estimator computes over a 512-byte window, so a periodic
    // signal must be at least that long to carry a period at all.
    let ab = "ab".repeat(350);
    demo_min("any strong byte-period", r"\F{period}", &ab, 1);
    demo_min("a specific byte-period", r"\F{period=2}", &ab, 1);
    // A change-point is a REGIME shift: the detector needs a few hundred bytes
    // of one texture before a different one, so the onset input is prose
    // followed by a high-entropy blob.
    let regime_shift =
        format!("{}8fK3#zQ9!mW2@vB7&pL4^dH6*tN1(rX5)yJ8", "the cat sat on the mat ".repeat(20));
    demo_min("onset: a spectral change-point inside the token", r"\F{onset}", &regime_shift, 1);

    println!("== 14. Typed value predicates: the value in the kind's own units ==\n");
    demo("a number by value, in a range", r"\N{500..599}", "GET 200 POST 503 PUT 599 GET 600", 2);
    demo(
        "an address inside a block",
        r"\I{in:10.0.0.0/8}",
        "src=10.20.30.40 dst=8.8.8.8 lan=10.0.255.1",
        2,
    );
    demo("a version by semver precedence", r"\V{>=2.0,<3}", "app 2.5.1 lib 1.4.2 tool 3.0.0", 1);
    demo("a size with the unit normalized", r"\Z{>1GB}", "disk 1.5GiB cache 512MB log 900KB", 1);
    demo("a duration against a unit", r"\R{>500ms}", "p50 80ms p99 2.5s max 3h20m", 2);
    demo(
        "a URL's host by glob",
        r"\U{host:*.internal}",
        "https://db.corp.internal/x http://example.com/internal",
        1,
    );
    demo("a card's issuer", r"\{creditcard}{issuer:visa}", "4111 1111 1111 1111 or 3782 822463 10005", 1);
    // The clock is fixed at 2026-09-15T00:00:00Z so the ages below are the
    // same on any day the example runs.
    trex::set_now(Some(1_789_430_400));
    demo(
        "a timestamp's age against the clock, in three forms",
        r"\T{age<24h}",
        "2026-09-14T22:00:00Z ok 2026-09-10 old Sep 14 23:30:00 warm",
        2,
    );
    trex::set_now(None);

    println!("== 15. Statistical anchors: zero-width, from a precomputed field ==\n");
    demo("@nested>1: at least two brackets deep", r"@nested>1 \W", "f(g(x))", 1);
    demo("@nested>=1: one bracket deep suffices", r"@nested>=1 \W", "f(g(x))", 2);
    demo_min(
        "@ambiguous: a vantage-dependent (garden-path) point",
        r"@ambiguous .",
        "the old man the boats and aaaaaaaaaaaaaaaa 9zK#q!X2mW8vB4&Q the end",
        1,
    );
    demo("@novel: first occurrence only", r"@novel \W", "cat dog cat", 2);
    demo("@echoed: recurring content only", r"@echoed \W", "cat dog cat bird", 2);
    // Seams are statistical, so they need real text (the Moby-Dick opening):
    // a cut is where the past stops predicting the future, which over
    // prose's token kinds is at the punctuation that ends a run of words.
    demo_min(
        "@seam: a predictive-segmentation break",
        r"@seam .",
        "Call me Ishmael. Some years ago, never mind how long precisely, having \
         little or no money in my purse, and nothing particular to interest me \
         on shore, I thought I would sail about a little and see the watery \
         part of the world.",
        1,
    );

    println!("== 16. Rewrite templates and typed sub-field accessors ==\n");
    demo_rewrite("whole match + capture", r"\N:n", "[${0}->${n}]", "a 7 b", "a [7->7] b");
    demo_rewrite("transform: upper", r"\E:e", "${e:upper}", "to bob@x.com", "to BOB@X.COM");
    demo_rewrite(
        "slice an IPv4 by octets",
        r"\I:ip",
        "${ip:octet1-2}.0.0/16",
        "from 192.168.5.9",
        "from 192.168.0.0/16",
    );
    demo_rewrite(
        "slice an IPv6 by groups (same atom)",
        r"\I:ip",
        "${ip:group1-3}",
        "addr 2001:db8:85a3:0:0:8a2e:370:7334",
        "addr 2001:db8:85a3",
    );
    demo_rewrite("URL host", r"\U:u", "${u:host}", "get https://example.com:8080/a?q=1", "get example.com");
    demo_rewrite("email domain, chained upper", r"\E:e", "${e:domain|upper}", "to bob@x.com", "to X.COM");
    demo_rewrite("version major", r"\V:v", "major=${v:major}", "rel 1.2.3-rc1", "rel major=1");
    demo_rewrite("timestamp year", r"\T:t", "${t:year}", "at 2026-07-06 noon", "at 2026 noon");
    demo_rewrite("path name + ext", r"\L:p", "${p:name}", "open /usr/bin/trex.exe", "open trex.exe");

    println!("every construct matched exactly as asserted.");
}

#[cfg(test)]
mod tests {
    /// Every claim here is an assertion inside `main`, so running it is the
    /// test. The manifest marks this example `test = true`, so `cargo test`
    /// runs it rather than only compiling it.
    #[test]
    fn every_demo_matches_what_it_claims() {
        super::main();
    }
}
