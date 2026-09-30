//! Predicates on what a token encodes: a JSON Web Token's header parameters
//! and payload claims, and a base64 blob's decoded bytes. Every case runs
//! through the front door and, forced by a committed choice, through the
//! set-reachability engine, and the two must agree.

use std::sync::{Mutex, MutexGuard};

use trex::{parse, scan};

/// The instant every test reads: 2026-09-15T00:00:00Z, UTC.
const NOW: i64 = 1_789_430_400;

/// The clock is a process-wide setting, so the tests that set it run one at
/// a time.
static CLOCK: Mutex<()> = Mutex::new(());

fn hold_clock() -> MutexGuard<'static, ()> {
    let guard = CLOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    trex::set_now(Some(NOW));
    trex::set_tz_offset(0);
    guard
}

/// `{"alg":"none"}` over `{"sub":"123","exp":1789430400}`: the unsigned
/// token, expiring at the clock's instant.
const NONE_ALG: &str = "eyJhbGciOiJub25lIn0.eyJzdWIiOiIxMjMiLCJleHAiOjE3ODk0MzA0MDB9.sig";

/// `{"alg":"HS256","typ":"JWT"}` over
/// `{"iss":"auth.internal","role":"admin","exp":1789516800,"ver":2}`.
const SIGNED: &str = concat!(
    "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.",
    "eyJpc3MiOiJhdXRoLmludGVybmFsIiwicm9sZSI6ImFkbWluIiwiZXhwIjoxNzg5NTE2ODAwLCJ2ZXIiOjJ9.",
    "dBjftJeZ4CVPmB92K27uhbUJU1p1r_wW1gFWFOEjXk"
);

/// `-----BEGIN RSA PRIVATE KEY-----`, entropy 3.38 bits a byte.
const PEM: &str = "LS0tLS1CRUdJTiBSU0EgUFJJVkFURSBLRVktLS0tLQ==";

/// `the quick brown fox jumps over the lazy dog`, entropy 4.39.
const PROSE: &str = "dGhlIHF1aWNrIGJyb3duIGZveCBqdW1wcyBvdmVyIHRoZSBsYXp5IGRvZw==";

/// The thirty bytes `00` to `1d`, each once, entropy 4.91.
const BINARY: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwd";

fn texts(pattern: &trex::ast::Pattern, bytes: &[u8]) -> Vec<String> {
    scan(pattern, bytes)
        .into_iter()
        .map(|s| String::from_utf8_lossy(&bytes[s.range()]).into_owned())
        .collect()
}

/// The matched texts of `pattern` over `input`, through the front door and
/// through the set engine, which must agree.
fn found(pattern: &str, input: &str) -> Vec<String> {
    let bytes = input.as_bytes();
    let pat = parse(pattern).unwrap_or_else(|e| panic!("{pattern}: {e:?}"));
    let direct = texts(&pat, bytes);
    let forced = parse(&format!("({pattern} |> [\\N && \\W])"))
        .unwrap_or_else(|e| panic!("forced {pattern}: {e:?}"));
    let through_set = texts(&forced, bytes);
    assert_eq!(direct, through_set, "{pattern}: the two engines disagree");
    direct
}

/// Both tokens in one line, as a log holds them.
fn tokens() -> String {
    format!("auth {NONE_ALG} then {SIGNED} done")
}

#[test]
fn a_header_parameter_reads_the_decoded_header() {
    let line = tokens();
    assert_eq!(found("\\{jwt}{alg:none}", &line), vec![NONE_ALG]);
    assert_eq!(found("\\{jwt}{alg:HS256}", &line), vec![SIGNED]);
    assert_eq!(found("\\{jwt}{alg:HS*}", &line), vec![SIGNED]);
    assert_eq!(found("\\{jwt}{header.alg:none}", &line), vec![NONE_ALG]);
    assert_eq!(found("\\{jwt}{typ:JWT}", &line), vec![SIGNED]);
    assert_eq!(found("\\{jwt}{alg!=none}", &line), vec![SIGNED]);
    // A parameter the header does not carry holds of nothing.
    assert!(found("\\{jwt}{kid:*}", &line).is_empty());
}

#[test]
fn a_claim_reads_the_decoded_payload() {
    let line = tokens();
    assert_eq!(found("\\{jwt}{sub:123}", &line), vec![NONE_ALG]);
    assert_eq!(found("\\{jwt}{role:admin}", &line), vec![SIGNED]);
    // A glob is what a claim compares by, so a host suffix reads as one.
    assert_eq!(found("\\{jwt}{iss:*.internal}", &line), vec![SIGNED]);
    assert!(found("\\{jwt}{iss:*.example}", &line).is_empty());
    // A number compares as a number, whichever half it was written in, and
    // `payload.x` names the half outright.
    assert_eq!(found("\\{jwt}{ver=2}", &line), vec![SIGNED]);
    assert_eq!(found("\\{jwt}{ver>1}", &line), vec![SIGNED]);
    assert_eq!(found("\\{jwt}{payload.role:admin}", &line), vec![SIGNED]);
    // A claim named like a header parameter is still reachable.
    assert_eq!(found("\\{jwt}{payload.sub:123}", &line), vec![NONE_ALG]);
}

#[test]
fn a_date_claim_compares_against_the_clock_and_an_instant() {
    let _clock = hold_clock();
    let line = tokens();
    // One token expires at the clock's instant, the other a day after it.
    assert!(found("\\{jwt}{exp<now}", &line).is_empty());
    assert_eq!(found("\\{jwt}{exp<=now}", &line), vec![NONE_ALG]);
    assert_eq!(found("\\{jwt}{exp>now}", &line), vec![SIGNED]);
    assert_eq!(found("\\{jwt}{exp<2026-09-16}", &line), vec![NONE_ALG]);
    assert_eq!(found("\\{jwt}{exp>=2026-09-16}", &line), vec![SIGNED]);
    // The clauses of one predicate are a conjunction whatever they cost.
    assert_eq!(found("\\{jwt}{alg:none,exp<=now}", &line), vec![NONE_ALG]);
    assert!(found("\\{jwt}{alg:none,exp>now}", &line).is_empty());
}

#[test]
fn a_halfs_own_text_and_a_blobs_text_read_as_text() {
    let line = tokens();
    assert_eq!(found("\\{jwt}{payload:*admin*}", &line), vec![SIGNED]);
    assert_eq!(found("\\{jwt}{header:*none*}", &line), vec![NONE_ALG]);
    assert_eq!(found("\\{jwt}{text:*auth.internal*}", &line), vec![SIGNED]);
    let blobs = format!("a {PEM} b {PROSE} c {BINARY} d");
    assert_eq!(found("\\{base64}{text:*BEGIN*}", &blobs), vec![PEM]);
    assert_eq!(found("\\{base64}{text:*lazy dog}", &blobs), vec![PROSE]);
    assert!(found("\\{base64}{text:*nothing*}", &blobs).is_empty());
}

#[test]
fn the_byte_readings_read_the_decoded_bytes() {
    let blobs = format!("a {PEM} b {PROSE} c {BINARY} d");
    // Entropy in bits a byte: 3.38, 4.39 and 4.91.
    assert_eq!(found("\\{base64}{bits<4}", &blobs), vec![PEM]);
    assert_eq!(found("\\{base64}{bits>4.5}", &blobs), vec![BINARY]);
    assert_eq!(found("\\{base64}{bits>4}", &blobs), vec![PROSE, BINARY]);
    assert_eq!(found("\\{base64}{bits>3,bits<4.5}", &blobs), vec![PEM, PROSE]);
    // Every blob decodes, so every one carries a texture class, and each
    // class the axis reports is one of the five it names.
    assert_eq!(found("\\{base64}{texture:*}", &blobs).len(), 3);
    let classes = ["prose", "code", "math", "data", "mixed"];
    let named: usize =
        classes.iter().map(|c| found(&format!("\\{{base64}}{{texture:{c}}}"), &blobs).len()).sum();
    assert_eq!(named, 3, "each blob's class is one the axis names");
    // The period a blob's bytes carry is what the axis reads of them, which
    // `src/decoded.rs` measures on bytes of a known row length; here it is
    // the field reaching them and ordering that is under test.
    assert_eq!(found("\\{base64}{period<1000}", &blobs).len(), 3);
    assert!(found("\\{base64}{period>=1000}", &blobs).is_empty());
}

#[test]
fn a_sub_pattern_matches_inside_the_decoded_bytes() {
    let mut shapes = trex::ShapeSet::new();
    shapes.declare_text("let begins = \"BEGIN\" \\W").expect("declares");
    let pattern = trex::parser::parse_with_shapes("\\{base64}{match:begins}", &shapes)
        .expect("the sub-pattern resolves");
    let blobs = format!("a {PEM} b {PROSE} c {BINARY} d");
    let spans = trex::engine::scan_with_shapes(&pattern, blobs.as_bytes(), &shapes);
    let got: Vec<&str> = spans.iter().map(|s| &blobs[s.start()..s.end()]).collect();
    assert_eq!(got, vec![PEM]);
    // The complement reads the other two.
    let not = trex::parser::parse_with_shapes("\\{base64}{match!=begins}", &shapes)
        .expect("the sub-pattern resolves");
    let spans = trex::engine::scan_with_shapes(&not, blobs.as_bytes(), &shapes);
    assert_eq!(spans.len(), 2);
    // A name no declaration holds is a parse error that says so.
    let err = trex::parser::parse_with_shapes("\\{base64}{match:nosuch}", &shapes)
        .expect_err("an undeclared name is refused");
    assert!(err.msg.contains("declared sub-pattern"), "{}", err.msg);
}

#[test]
fn a_token_that_does_not_decode_satisfies_nothing() {
    // A JWT whose payload is not a JSON object, and a blob that is not text.
    let odd = format!("x eyJhbGciOiJub25lIn0.bm90anNvbg.sig y {BINARY} z");
    assert!(found("\\{jwt}{role:admin}", &odd).is_empty());
    assert!(found("\\{base64}{text:*BEGIN*}", &odd).is_empty());
    // The kinds still match without a predicate, so the tokens are there.
    assert_eq!(found("\\{jwt}", &odd).len(), 1);
    assert_eq!(found("\\{base64}", &odd).len(), 1);
}

#[test]
fn a_field_the_kind_does_not_have_is_a_parse_error() {
    let err = parse("\\{base64}{alg:none}").expect_err("a blob has no claims");
    assert!(err.msg.contains("no field"), "{}", err.msg);
    // A token names its own claims, so a name that is no field of the kind
    // is a claim rather than an error, and holds of a token carrying it. A
    // dotted name past the half is one key, since an object may hold one.
    assert_eq!(found("\\{jwt}{bits:7}", &tokens()).len(), 0);
    assert_eq!(found("\\{jwt}{header.alg.deep:x}", &tokens()).len(), 0);
}
