//! A set scanned over a lex its caller already holds. A streaming push lexes
//! its retained buffer and then hands those tokens to the set rather than
//! paying for a second lex of the same bytes, so the lent tokens must answer
//! exactly what the set's own lex answers. A set declaring shapes goes on
//! taking a lex of its own, since the lent one was taken without those
//! declarations and carries boundaries they never made.

use trex::PatternSet;
use trex::engine::Span;

/// A spread over the routes a set member can take: byte-routable literals
/// present and absent, a word then punctuation, a byte pattern, a kind
/// sequence, and a balanced group only the set engine advances.
const SOURCES: &[&str] = &[
    "\"alpha\"",
    "\"zzzqqq\"",
    "\\W",
    "\\N",
    "\\W \"=\"",
    "`cond_[0-9]+`",
    "\"let\" \\W \"=\"",
    "\\B(\\W)",
];

/// Lines of four shapes, to at least `bytes`.
fn corpus_of_at_least(bytes: usize) -> Vec<u8> {
    let mut s = String::new();
    let mut i = 0usize;
    while s.len() < bytes {
        match i % 4 {
            0 => s.push_str(&format!("let value_{i} = {} ;\n", i * 37)),
            1 => s.push_str(&format!("call_{i}(alpha, beta, {i}) ;\n")),
            2 => s.push_str(&format!("key_{i}: item_{i}, item_{} ;\n", i + 1)),
            _ => s.push_str(&format!("if (cond_{i}) {{ do_{i}(x) ; }}\n")),
        }
        i += 1;
    }
    s.into_bytes()
}

fn built() -> PatternSet {
    PatternSet::new(SOURCES.iter().map(|s| trex::parse(s).expect("pattern parses")).collect())
}

/// The set asked over its own lex and over the lent one, from the start and
/// from the positions a stream resumes at: the lent tokens are cut at `at` by
/// the partition the set's own lex is cut at, so a resumed scan is where a
/// disagreement between the two would show.
fn the_two_agree(input: &[u8], toks: &[trex::token::Token]) {
    let set = built();
    let from_the_start: Vec<(usize, Span)> = set.scan_from(input, 0);
    assert!(!from_the_start.is_empty(), "the corpus reaches the members");
    for at in [0, 1, 137, input.len() / 3, input.len() / 2, input.len()] {
        let want = set.scan_from(input, at);
        assert_eq!(set.scan_from_over(input, at, toks), want, "at {at} of {}", input.len());
    }
}

#[test]
fn a_lent_lex_answers_what_the_sets_own_lex_answers() {
    let input = corpus_of_at_least(8 * 1024);
    assert!(
        input.len() < trex::parallel_lex::active_parallel_threshold(),
        "this corpus is the serial-lex arm"
    );
    the_two_agree(&input, &trex::lexer::lex(&input));
}

#[test]
fn a_lent_serial_lex_answers_what_a_parallel_one_answers() {
    // Above the threshold the set's own lex is the parallel one while the lex
    // a push lends is serial. The two streams are documented byte-identical,
    // and this is the set reading them as such: what a stream lends is a
    // different lexer's output from what the same scan would have taken.
    let input = corpus_of_at_least(trex::parallel_lex::active_parallel_threshold() * 2);
    assert!(input.len() > trex::parallel_lex::active_parallel_threshold());
    the_two_agree(&input, &trex::lexer::lex(&input));
}

#[test]
fn a_shaped_set_reads_its_own_lex_and_not_the_lent_one() {
    // The shape fuses bytes the plain lexer splits, so a set reading the lent
    // tokens here would walk boundaries its declarations never made.
    let text = "shape ticket = `[A-Z]{2,4}-\\d{1,4}`\nlet cited = \\{ticket}\n";
    let mut shapes = trex::ShapeSet::new();
    let set = PatternSet::from_text(text, &mut shapes).expect("the file declares a set");
    let input = b"see AB-12 and XYZ-9 now\n";
    let plain = trex::lexer::lex(input);
    let want = set.scan_from(input, 0);
    assert_eq!(want.len(), 2, "both tickets are found under the shape: {want:?}");
    assert_eq!(set.scan_from_over(input, 0, &plain), want);
}
