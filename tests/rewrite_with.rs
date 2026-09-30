//! `rewrite_with`: a replacement decided per match by a closure that reads
//! the match, its registers and the typed slices a template reference
//! names, through the same reference syntax a template uses.

use trex::{Field, Reference, captures, parse, rewrite_n_with, rewrite_with, scan};

#[test]
fn the_closure_reads_the_match_its_registers_and_typed_slices() {
    let pat = parse(r"\E:e").expect("valid pattern");
    let input = b"mail bob@x.com and amy@y.org now";
    let mut seen = Vec::new();
    let out = rewrite_with(&pat, input, |m| {
        seen.push((m.start(), m.end(), m.text().into_owned()));
        assert_eq!(m.group("e"), Some(m.as_bytes()));
        assert_eq!(m.names(), &["e".to_string()][..]);
        assert_eq!(m.input(), input);
        assert_eq!(m.inner().start, m.start());
        let user = m.get("e:user").expect("a user slice");
        let domain = m.get("e:domain").expect("a domain slice");
        format!("{}:{}@{}", seen.len(), user.to_uppercase(), domain.len())
    });
    assert_eq!(out, b"mail 1:BOB@5 and 2:AMY@5 now");
    assert_eq!(seen, vec![(5, 14, "bob@x.com".to_string()), (19, 28, "amy@y.org".to_string())]);
}

#[test]
fn a_reference_reads_as_a_template_renders_it() {
    let pat = parse(r"\W:w \I:ip").expect("valid pattern");
    let input = b"from 10.1.2.3 to 192.168.0.1";
    let out = rewrite_with(&pat, input, |m| {
        [
            m.get("0").expect("the whole match"),
            m.get("ip:octet1-2").expect("two octets"),
            m.get("1:upper").expect("the first bound register, by position"),
            m.get("2:last1").expect("the second, by position"),
            m.get("w:trim|upper").expect("a chain"),
        ]
        .join("|")
    });
    assert_eq!(out, b"from 10.1.2.3|10.1|FROM|3|FROM to 192.168.0.1|192.168|TO|1|TO");
    // The same references over the matches `captures` resolves.
    let matches = captures(&pat, input, &scan(&pat, input));
    let bound = pat.capture_names();
    let r = Reference::parse("ip:octet4", &bound).expect("a reference");
    assert_eq!(r.field(), &Field::Register("ip".to_string()));
    assert_eq!(r.read(&matches[0], input), "3");
    assert_eq!(Reference::parse("0", &bound).expect("the whole").field(), &Field::Whole);
    assert_eq!(Reference::parse("w:lower", &bound).expect("a reference").apply("FROM"), "from");
    // A slice the value does not have renders nothing, as a template does.
    assert_eq!(Reference::parse("w:domain", &bound).expect("a reference").read(&matches[0], input), "");
}

#[test]
fn a_bad_reference_is_an_error_naming_it() {
    let pat = parse(r"\E:e").expect("valid pattern");
    let input = b"bob@x.com";
    let mut errors = Vec::new();
    let out = rewrite_with(&pat, input, |m| {
        errors.push(m.get("nope").expect_err("an unbound name").msg);
        errors.push(m.get("e:bogus").expect_err("an unknown accessor").msg);
        errors.push(m.get("2").expect_err("a position past the last").msg);
        "-"
    });
    assert_eq!(out, b"-");
    assert!(errors[0].contains("binds no such capture"), "{}", errors[0]);
    assert!(errors[1].contains("unknown accessor ':bogus'"), "{}", errors[1]);
    assert!(errors[2].contains("binds 1 capture"), "{}", errors[2]);
}

#[test]
fn a_pattern_binding_nothing_still_hands_over_the_match() {
    let pat = parse(r"\N").expect("valid pattern");
    let input = b"a 1 b 22 c 333";
    let out = rewrite_with(&pat, input, |m| {
        assert!(m.names().is_empty());
        assert_eq!(m.group("x"), None);
        m.get("0:last1").expect("the whole match sliced")
    });
    assert_eq!(out, b"a 1 b 2 c 3");
    // The closure may return any bytes: a slice, a string, a vector.
    assert_eq!(rewrite_with(&pat, input, |_| "-"), b"a - b - c -");
    assert_eq!(rewrite_with(&pat, input, |m| m.as_bytes().to_vec()), input);
    assert_eq!(rewrite_with(&pat, b"none here", |_| "-"), b"none here");
}

#[test]
fn the_first_n_matches_are_rewritten_and_the_rest_left() {
    let pat = parse(r"\N").expect("valid pattern");
    let input = b"1 2 3 4";
    assert_eq!(rewrite_n_with(&pat, input, 1, |m| format!("<{}>", m.text())), b"<1> 2 3 4");
    assert_eq!(rewrite_n_with(&pat, input, 2, |_| "x"), b"x x 3 4");
    assert_eq!(rewrite_n_with(&pat, input, 0, |_| "x"), input);
    assert_eq!(rewrite_n_with(&pat, input, 9, |_| "x"), b"x x x x");
}
