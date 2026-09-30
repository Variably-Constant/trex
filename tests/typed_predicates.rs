//! Typed value predicates over real inputs, through the engines a scan takes:
//! the front door (routes and the single-pass engine) and, forced by a
//! committed choice, the set-reachability engine. The two must agree on every
//! case, and each case says what it expects to find.

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
    // `|>` routes a pattern to the set engine, and a branch no token can
    // satisfy leaves the language unchanged.
    let forced = parse(&format!("({pattern} |> [\\N && \\W])"))
        .unwrap_or_else(|e| panic!("forced {pattern}: {e:?}"));
    let through_set = texts(&forced, bytes);
    assert_eq!(direct, through_set, "{pattern}: the two engines disagree");
    direct
}

/// A directory of set files under the temp directory, removed when dropped.
struct Sets {
    dir: std::path::PathBuf,
}

impl Sets {
    fn new(name: &str) -> Sets {
        let dir = std::env::temp_dir().join(format!("trex/sets-{name}-{}", std::process::id()));
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => panic!("cannot clear {}: {e}", dir.display()),
        }
        std::fs::create_dir_all(&dir).expect("create the set directory");
        Sets { dir }
    }

    /// Write a set file and return its path with forward slashes, which a
    /// predicate body carries as it is.
    fn write(&self, name: &str, text: &str) -> String {
        let path = self.dir.join(name);
        std::fs::write(&path, text).expect("write a set file");
        path.to_string_lossy().replace('\\', "/")
    }
}

impl Drop for Sets {
    fn drop(&mut self) {
        match std::fs::remove_dir_all(&self.dir) {
            Ok(()) => {}
            Err(e) => eprintln!("cannot remove {}: {e}", self.dir.display()),
        }
    }
}

#[test]
fn a_set_read_from_a_file_is_typed_membership() {
    let sets = Sets::new("typed");
    let cidrs = sets.write("cidrs.txt", "# blocks\n10.0.0.0/8\n192.168.1.0/24\n2001:db8::/32\n172.16.5.7\n");
    let log = "from 10.4.5.6 and 192.168.2.1 and 192.168.1.9 and 172.16.5.7 and 2001:db8::1 and 8.8.8.8";
    assert_eq!(
        found(&format!("\\I{{in:@{cidrs}}}"), log),
        vec!["10.4.5.6", "192.168.1.9", "172.16.5.7", "2001:db8::1"]
    );
    let words = sets.write("words.txt", "alpha\nGamma\n");
    let text = "alpha beta gamma Gamma ALPHA";
    assert_eq!(found(&format!("\\W{{in:@{words}}}"), text), vec!["alpha", "Gamma"]);
    assert_eq!(
        found(&format!("\\W{{in:case:@{words}}}"), text),
        vec!["alpha", "gamma", "Gamma", "ALPHA"]
    );
    let domains = sets.write("blocklist.txt", "evil.example\nBAD.test\n");
    let mail = "bob@evil.example amy@good.example x@bad.test";
    assert_eq!(
        found(&format!("\\E{{domain:@{domains}}}"), mail),
        vec!["bob@evil.example", "x@bad.test"]
    );
    assert_eq!(found(&format!("\\E{{domain!=@{domains}}}"), mail), vec!["amy@good.example"]);
    let ports = sets.write("ports.txt", "22\n443\n8080\n");
    assert_eq!(found(&format!("\\N{{in:@{ports}}}"), "on 22 80 443 8080 9000"), vec!["22", "443", "8080"]);
    let hosts = sets.write("hosts.txt", "api.internal\n");
    assert_eq!(
        found(&format!("\\U{{host:@{hosts}}}"), "http://api.internal/x and http://www.example/y"),
        vec!["http://api.internal/x"]
    );
    let blocks = sets.write("blocks.txt", "10.0.0.0/8\n");
    assert_eq!(found(&format!("\\C{{in:@{blocks}}}"), "10.1.0.0/16 and 11.0.0.0/8"), vec!["10.1.0.0/16"]);
}

#[test]
fn a_missing_or_malformed_set_is_a_parse_error() {
    let sets = Sets::new("bad");
    let missing = sets.dir.join("nope.txt").to_string_lossy().replace('\\', "/");
    assert!(parse(&format!("\\W{{in:@{missing}}}")).is_err());
    let bad = sets.write("bad.txt", "10.0.0.0/8\nnot an address\n");
    let e = parse(&format!("\\I{{in:@{bad}}}")).unwrap_err();
    assert!(e.msg.contains("line 2"), "{}", e.msg);
    let words = sets.write("w.txt", "a\n");
    assert!(parse(&format!("\\N{{in:case:@{words}}}")).is_err(), "a group folds text alone");
    assert!(parse(&format!("\\N{{in:@{words}}}")).is_err(), "a word is not a number");
}

#[test]
fn a_set_beside_a_pattern_file_is_read_from_there() {
    let sets = Sets::new("beside");
    sets.write("levels.txt", "ERROR\nFATAL\n");
    let lib = sets.write("defs.trex", "let bad = \\W{in:@levels.txt}\n");
    let mut set = trex::ShapeSet::new();
    set.declare_file(std::path::Path::new(&lib)).expect("declares");
    let pat = trex::parser::parse_with_shapes("\\{bad}", &set).expect("parses");
    assert_eq!(texts(&pat, b"INFO ok ERROR down FATAL"), vec!["ERROR", "FATAL"]);
}

#[test]
fn a_large_word_list_is_one_lookup_a_token() {
    let sets = Sets::new("large");
    let mut list = String::new();
    for i in 0..100_000 {
        list.push_str(&format!("word{i}\n"));
    }
    let words = sets.write("many.txt", &list);
    let mut text = String::new();
    for i in (0..20_000).step_by(7) {
        text.push_str(&format!("word{i} other{i} "));
    }
    let pat = parse(&format!("\\W{{in:@{words}}}")).expect("parses");
    assert_eq!(scan(&pat, text.as_bytes()).len(), (0..20_000).step_by(7).count());
}

#[test]
fn numbers_compare_by_value_and_ranges_are_inclusive() {
    let _clock = hold_clock();
    let log = "GET /api/a 200 12ms\nPOST /api/b 503 3s\nGET /api/c 404 80ms\nPUT /api/d 599 1s\nGET /api/e 600 5ms\n";
    assert_eq!(found("\\N{500..599}", log), vec!["503", "599"]);
    assert_eq!(found("\\N{>=400,<500}", log), vec!["404"]);
    assert_eq!(found("\\N{!=200}", log), vec!["503", "404", "599", "600"]);
    assert_eq!(
        found("\\W \\L \\N{>=500} \\R{>=1s}", log),
        vec!["POST /api/b 503 3s", "PUT /api/d 599 1s"]
    );
    // The value, not the order of magnitude: a value predicate and a magnitude
    // predicate on the same input answer differently.
    let big = "12 5000000000 300";
    assert_eq!(found("\\N{>300}", big), vec!["5000000000"]);
    assert_eq!(found("\\N{mag>2}", big), vec!["5000000000", "300"]);
}

#[test]
fn addresses_are_compared_as_addresses() {
    let _clock = hold_clock();
    let log = "src=10.20.30.40 dst=192.168.1.9 via=fe80::1 ext=8.8.8.8 lan=10.0.255.1\n";
    assert_eq!(found("\\I{in:10.0.0.0/8}", log), vec!["10.20.30.40", "10.0.255.1"]);
    assert_eq!(found("\\I{v6}", log), vec!["fe80::1"]);
    assert_eq!(found("\\I{v4,!=8.8.8.8}", log), vec!["10.20.30.40", "192.168.1.9", "10.0.255.1"]);
    assert_eq!(found("\"src\" \"=\" \\I{in:10.0.0.0/8}", log), vec!["src=10.20.30.40"]);
    let blocks = "allow 10.0.0.0/8 deny 10.1.0.0/16 allow 192.168.0.0/24";
    assert_eq!(found("\\C{contains:10.1.2.3}", blocks), vec!["10.0.0.0/8", "10.1.0.0/16"]);
    assert_eq!(found("\\C{prefix>=16}", blocks), vec!["10.1.0.0/16", "192.168.0.0/24"]);
}

#[test]
fn urls_emails_and_paths_read_their_fields() {
    let _clock = hold_clock();
    let text = "see https://db.corp.internal:5432/x and http://example.com/internal?token=abc \
                mail root@corp.internal or bob@example.org, logs in /var/log/app.log and ./notes.txt";
    assert_eq!(found("\\U{host:*.internal}", text), vec!["https://db.corp.internal:5432/x"]);
    assert_eq!(found("\\U{port>1024}", text), vec!["https://db.corp.internal:5432/x"]);
    assert_eq!(found("\\U{port=80}", text), vec!["http://example.com/internal?token=abc"]);
    assert_eq!(found("\\U{query:*token=*}", text), vec!["http://example.com/internal?token=abc"]);
    assert_eq!(found("\\E{domain:corp.internal}", text), vec!["root@corp.internal"]);
    assert_eq!(found("\\E{user:bob}", text), vec!["bob@example.org"]);
    assert_eq!(found("\\L{ext:log}", text), vec!["/var/log/app.log"]);
    assert_eq!(found("\\L{name:notes.*}", text), vec!["./notes.txt"]);
}

#[test]
fn timestamps_compare_as_instants_in_every_form() {
    let _clock = hold_clock();
    let log = "2026-09-14T22:00:00Z start\nSep 14 23:30:00 warm\n15/Sep/2026:01:00:00 +0200 request\n\
               2026-09-10 08:00:00 old\n2024-01-01 ancient\n";
    assert_eq!(
        found("\\T{age<24h}", log),
        vec!["2026-09-14T22:00:00Z", "Sep 14 23:30:00", "15/Sep/2026:01:00:00 +0200"]
    );
    assert_eq!(found("\\T{age>1y}", log), vec!["2024-01-01"]);
    assert_eq!(found("\\T{2026-09-01..2026-09-14}", log), vec!["2026-09-10 08:00:00"]);
    assert_eq!(
        found("\\T{<now,>=2026-09-14T23:00:00Z}", log),
        vec!["Sep 14 23:30:00", "15/Sep/2026:01:00:00 +0200"]
    );
    assert_eq!(found("\\T{year=2024}", log), vec!["2024-01-01"]);
    assert_eq!(
        found("\\T{hour>=22} \\W", log),
        vec!["2026-09-14T22:00:00Z start", "Sep 14 23:30:00 warm"]
    );
}

#[test]
fn a_naive_timestamp_takes_the_zone_the_process_sets() {
    let _clock = hold_clock();
    let log = "2026-09-14T23:30:00 late\n";
    let pat = parse("\\T{age<1h}").unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(scan(&pat, log.as_bytes()).len(), 1, "read as UTC it is half an hour old");
    // Read as +02:00 it is 21:30Z, two and a half hours old.
    trex::set_tz_offset(7200);
    let pat = parse("\\T{age<1h}").unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(scan(&pat, log.as_bytes()).len(), 0);
    trex::set_tz_offset(0);
}

#[test]
fn versions_sizes_durations_money_cards_and_phones() {
    let _clock = hold_clock();
    let text = "app 2.5.1 lib v1.4.2 tool 3.0.0-rc.1 core 1.9.9; disk 1.5GiB cache 512MB log 900KB; \
                p50 80ms p99 2.5s max 3h20m; paid $1,234.56 then $99.00; \
                card 4111 1111 1111 1111 or 3782 822463 10005; call +44 20 7123 4567 or 212-555-1234";
    // A pre-release of 3.0.0 lies below 3.0.0, so it is inside `<3`.
    assert_eq!(found("\\V{>=2.0,<3}", text), vec!["2.5.1", "3.0.0-rc.1"]);
    assert_eq!(found("\\V{major=1,minor>=4}", text), vec!["v1.4.2", "1.9.9"]);
    assert_eq!(found("\\V{pre:rc*}", text), vec!["3.0.0-rc.1"]);
    assert_eq!(found("\\Z{>1GB}", text), vec!["1.5GiB"]);
    assert_eq!(found("\\Z{<1MB}", text), vec!["900KB"]);
    assert_eq!(found("\\R{>500ms}", text), vec!["2.5s", "3h20m"]);
    assert_eq!(found("\\${>1000}", text), vec!["$1,234.56"]);
    assert_eq!(found("\\{creditcard}{issuer:visa}", text), vec!["4111 1111 1111 1111"]);
    assert_eq!(found("\\{card}{issuer:amex,len=15}", text), vec!["3782 822463 10005"]);
    assert_eq!(found("\\{phone}{cc=44}", text), vec!["+44 20 7123 4567"]);
    assert_eq!(found("\\{phone}{cc=1}", text), vec!["212-555-1234"]);
}

#[test]
fn a_back_reference_holds_under_a_typed_relation() {
    let _clock = hold_clock();
    let logins = "login 10.0.0.5 ok; login 10.0.0.77 ok; login 10.0.9.1 ok; login fe80::1 ok; login fe80::9 ok";
    assert_eq!(
        found("\"login\" \\I:a .*? \"login\" =subnet a", logins),
        vec!["login 10.0.0.5 ok; login 10.0.0.77", "login fe80::1 ok; login fe80::9"]
    );
    assert_eq!(found("\\I:a .*? =subnet/16 a", "a 10.1.0.1 b 10.1.200.3"), vec!["10.1.0.1 b 10.1.200.3"]);
    assert!(found("\\I:a .*? =subnet a", "a 10.1.0.1 b 10.1.200.3").is_empty());
    assert_eq!(
        found("\\E:e .*? =domain e", "bob@x.com ann@y.org eve@X.COM"),
        vec!["bob@x.com ann@y.org eve@X.COM"]
    );
    assert_eq!(
        found(
            "\\T:t .*? =day t",
            "2026-09-15T23:30:00+02:00 start 2026-09-16T00:10:00+02:00 mid 2026-09-15 01:00:00 end"
        ),
        vec!["2026-09-15T23:30:00+02:00 start 2026-09-16T00:10:00+02:00 mid 2026-09-15 01:00:00"]
    );
    assert_eq!(found("\\V:v .*? =major v", "1.4.2 then 2.0.0 then 1.9.9"), vec!["1.4.2 then 2.0.0 then 1.9.9"]);
    assert_eq!(found("\\N:n .*? =magnitude n", "size 1234 and 5000 and 99"), vec!["1234 and 5000"]);
    assert_eq!(
        found("\\{card}:c .*? =issuer c", "4111 1111 1111 1111 then 3782 822463 10005 then 4012888888881881"),
        vec!["4111 1111 1111 1111 then 3782 822463 10005 then 4012888888881881"]
    );
}

#[test]
fn a_back_reference_holds_up_to_a_typed_representation() {
    let _clock = hold_clock();
    assert_eq!(
        found("\\I:a .*? =ip a", "2001:DB8::1 x 2001:db8:0:0:0:0:0:1 y"),
        vec!["2001:DB8::1 x 2001:db8:0:0:0:0:0:1"]
    );
    assert_eq!(
        found("\\U:u .*? =url u", "HTTP://Example.COM:80/a then http://example.com/a"),
        vec!["HTTP://Example.COM:80/a then http://example.com/a"]
    );
    assert_eq!(
        found("\\T:t .*? =time t", "2026-09-15T02:00:00+02:00 then 2026-09-15 00:00:00"),
        vec!["2026-09-15T02:00:00+02:00 then 2026-09-15 00:00:00"]
    );
    assert_eq!(found("\\L:f .*? =path f", "C:\\a\\b then C:/a/b"), vec!["C:\\a\\b then C:/a/b"]);
    assert_eq!(found("(?orbit:fold \"cafe\")", "Caf\u{E9} CAFE cafe cafes"), vec!["Caf\u{E9}", "CAFE", "cafe"]);
    assert_eq!(found("(?orbit:numeric \"1e3\")", "1000 1000.0 999"), vec!["1000", "1000.0"]);
    assert_eq!(found("\\N:n .*? =numeric n", "1000 x 1000.00"), vec!["1000 x 1000.00"]);
}

#[test]
fn a_predicate_composes_with_binding_quantifiers_and_classes() {
    let _clock = hold_clock();
    let text = "ports 22 80 443 8080 8443 and words alpha beta gamma";
    assert_eq!(found("\\N{>1024}+", text), vec!["8080 8443"]);
    assert_eq!(found("\\W{len>4}", text), vec!["ports", "words", "alpha", "gamma"]);
    let pat = parse("\\N{>1024}:p").unwrap_or_else(|e| panic!("{e:?}"));
    let spans = scan(&pat, text.as_bytes());
    let caps = trex::captures(&pat, text.as_bytes(), &spans);
    assert_eq!(caps.len(), 2);
    assert_eq!(caps[0].names(), ["p".to_string()].as_slice());
    assert_eq!(&text.as_bytes()[caps[0].captures()[0].range()], b"8080");
}
