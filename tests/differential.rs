//! Differential oracle: on its pure-regular subset, trex must
//! produce the same match spans as an independent regular-
//! expression engine.
//!
//! trex matches whole typed tokens, so the comparison uses regex
//! patterns whose semantics coincide with the typed atom (for
//! example `\bfoo\b` for the whole-token literal `"foo"`). Where
//! the semantics coincide, the match byte spans must be identical.

use regex::Regex;
use trex::parse;

fn trex_spans(pattern: &str, input: &str) -> Vec<(usize, usize)> {
    let p = parse(pattern).expect("trex pattern parses");
    trex::scan(&p, input.as_bytes())
        .iter()
        .map(|m| (m.start(), m.end()))
        .collect()
}

fn regex_spans(re: &str, input: &str) -> Vec<(usize, usize)> {
    let r = Regex::new(re).expect("oracle regex compiles");
    r.find_iter(input).map(|m| (m.start(), m.end())).collect()
}

fn assert_agrees(trex_pat: &str, regex_pat: &str, inputs: &[&str]) {
    for inp in inputs {
        assert_eq!(
            trex_spans(trex_pat, inp),
            regex_spans(regex_pat, inp),
            "disagreement on input {inp:?} (trex {trex_pat:?} vs regex {regex_pat:?})"
        );
    }
}

#[test]
fn number_atom_agrees_with_oracle() {
    // The equivalence holds only on token-aligned inputs: trex `\N`
    // is a number token, so a digit run fused to letters ("x12")
    // tokenizes as a word and is not a number, unlike byte-regex
    // `\d+`. These inputs keep numbers bounded by whitespace or
    // punctuation so the two granularities coincide.
    assert_agrees(
        "\\N",
        r"\d+(?:\.\d+)?",
        &[
            "ab 12 3.5 cd",
            "12.34 56 78",
            "no digits here",
            "007 then 4.2 end",
            "",
            "3.14159",
        ],
    );
}

#[test]
fn binding_a_token_does_not_move_the_spans_it_reports() {
    // The comparison bench pairs a bound pattern with its unbound twin to read
    // what the binding costs, and that only divides out if the two report the
    // same spans. A named group wrapping the whole expression is the oracle's
    // own way of saying the same thing, so both halves are checked here.
    assert_agrees(
        "\\W:w",
        r"(?P<w>[A-Za-z_][A-Za-z0-9_]*)",
        &["alpha beta", "let value_1 = 37 ;", "", "no_digits", "a b c"],
    );
    assert_agrees(
        "\\N:n",
        r"(?P<n>\d+(?:\.\d+)?)",
        &["ab 12 3.5 cd", "12.34 56 78", "no digits here", "007 then 4.2 end", "", "3.14159"],
    );
    for (bound, bare) in [("\\W:w", "\\W"), ("\\N:n", "\\N")] {
        for inp in ["alpha beta 12", "let value_1 = 37 ;", "", "3.14 x", "007 then 4.2 end"] {
            assert_eq!(
                trex_spans(bound, inp),
                trex_spans(bare, inp),
                "binding moved the spans of {bound} against {bare} on {inp:?}"
            );
        }
    }
}

#[test]
fn word_atom_agrees_with_oracle() {
    assert_agrees(
        "\\W",
        r"[A-Za-z_][A-Za-z0-9_]*",
        &[
            "ab 12 3.5 cd",
            "a_b-c d12 9x",
            "CONST value_2 ok",
            "   ",
            "hello world",
        ],
    );
}

#[test]
fn whole_token_literal_agrees_with_word_boundary_regex() {
    assert_agrees(
        "\"foo\"",
        r"\bfoo\b",
        &["foo food foo", "foofoo", "a foo b", "barfoo foo bar"],
    );
}

#[test]
fn number_then_word_sequence_agrees() {
    // A number and a word across a space match the same span in both
    // engines. The words here are ones no unit table holds: a unit symbol
    // after a number is that quantity's, and trex reads the two as one
    // token where the byte regex reads a number and a word.
    assert_agrees("\\N \\W", r"\d+(?:\.\d+)?\s+[A-Za-z_][A-Za-z0-9_]*", &[
        "weight 12 crates total",
        "10 items then 20 boxes",
    ]);
}

/// Inputs whose token boundaries are unambiguous, so the token grain and the
/// byte grain read the same spans. Every case below runs over all of them.
const TOKEN_ALIGNED: &[&str] = &[
    "",
    "a",
    "12",
    "alpha beta gamma",
    "a 1 b 2 c 3",
    "one two 33 four 5",
    "x y z",
    "log 404 error 500 done",
    "a q b q",
    "no match in this line at all",
];

#[test]
fn quantifiers_agree_with_the_oracle() {
    // The Rust regex crate is leftmost-first, which is the semantics `|` and
    // the greedy quantifiers were built to match, so these are a check on
    // trex's reading of that rather than on the shapes alone.
    let word = r"[A-Za-z_][A-Za-z0-9_]*";
    assert_agrees("\\W+", &format!(r"{word}(?:\s+{word})*"), TOKEN_ALIGNED);
    assert_agrees("\\N+", r"\d+(?:\.\d+)?(?:\s+\d+(?:\.\d+)?)*", TOKEN_ALIGNED);
    // A bare `\W?` is not comparable: regex reports an empty match at every
    // position an optional can match nothing, and trex reports only non-empty
    // spans. The optional is exercised inside a sequence instead, where both
    // engines have something to anchor it against.
    assert_agrees("\\W? \\N", &format!(r"(?:{word}\s+)?\d+(?:\.\d+)?"), TOKEN_ALIGNED);
}

#[test]
fn alternation_agrees_with_the_oracle() {
    // `|` is leftmost-first in both, so the earlier branch wins where both
    // could match and the later one still wins when the first cannot.
    let word = r"[A-Za-z_][A-Za-z0-9_]*";
    let num = r"\d+(?:\.\d+)?";
    assert_agrees("\\N | \\W", &format!(r"{num}|{word}"), TOKEN_ALIGNED);
    assert_agrees("\\W | \\N", &format!(r"{word}|{num}"), TOKEN_ALIGNED);
}

#[test]
fn lazy_and_greedy_agree_with_the_oracle() {
    // The case that separates the two leans: a repeated terminator, where
    // greedy runs to the last and lazy stops at the first. Both engines are
    // leftmost-first, so the spans must match exactly.
    // `.` is not usable as the oracle's counterpart here: regex may begin a
    // match inside a whitespace run, where trex resumes at the next token
    // start, so the spans differ by the leading space rather than by the lean.
    // A token atom keeps both engines on the same boundaries.
    let word = r"[A-Za-z_][A-Za-z0-9_]*";
    assert_agrees("\\W* \"q\"", &format!(r"(?:{word}\s+)*q\b"), TOKEN_ALIGNED);
    assert_agrees("\\W*? \"q\"", &format!(r"(?:{word}\s+)*?q\b"), TOKEN_ALIGNED);
}

#[test]
fn input_anchors_agree_with_the_oracle() {
    // trex anchors are prefix operators and regex's are postfix, so the
    // equivalent regex puts \A and \z on the far side of the atom.
    let word = r"[A-Za-z_][A-Za-z0-9_]*";
    assert_agrees("\\A \\W", &format!(r"\A{word}"), TOKEN_ALIGNED);
    assert_agrees("\\z \\W", &format!(r"{word}\z"), TOKEN_ALIGNED);
}

#[test]
fn bounded_repeat_agrees_with_the_oracle() {
    let word = r"[A-Za-z_][A-Za-z0-9_]*";
    assert_agrees("\\W{2}", &format!(r"{word}\s+{word}"), TOKEN_ALIGNED);
    assert_agrees("\\W{1,2}", &format!(r"{word}(?:\s+{word})?"), TOKEN_ALIGNED);
}
