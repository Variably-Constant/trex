//! A pattern built with its readings named, rather than parsed and taken as
//! it comes.
//!
//! The regex crate's `RegexBuilder` carries twelve knobs. Most of them decide
//! how bytes are grouped into the things a pattern matches - whether a dot
//! crosses a newline, what counts as a line terminator, whether a character
//! class folds case, whether whitespace in the pattern is significant. Over
//! tokens those questions are already answered, and not by a pattern flag:
//! the lexer decides what a token is, and it decided before any pattern was
//! compiled.
//!
//! So this builder carries the knobs that still mean something once the
//! alphabet is tokens:
//!
//! - **case folding**, because comparing a literal to a token is a comparison
//!   and an equivalence can be chosen for it. It is [`OrbitGroup::Case`],
//!   the same one `"Cat"~case` names per literal, applied to every literal at
//!   once.
//! - **swapped greed**, because a quantifier's preference is expressed by the
//!   order the engine queues its threads, which is a property of the pattern
//!   and not of the alphabet.
//! - **nesting depth**, because the parser descends recursively, so depth is
//!   stack and a caller on a small stack may want a smaller ceiling than
//!   [`crate::parser::NEST_LIMIT`].
//! - **the empty-loop reading**, which decides what a repetition whose body
//!   matches nothing does, and which trex already exposes because the two
//!   readings genuinely disagree.
//!
//! The other regex knobs have no counterpart here, and it is worth being
//! exact about why rather than listing them as missing. `ignore_whitespace`
//! is meaningless because whitespace already separates atoms in a trex
//! pattern rather than being matchable text. `dot_matches_new_line`,
//! `multi_line`, `crlf` and `line_terminator` are lexer questions: `.` is one
//! token and never a byte, and `^` anchors to a line the lexer has already
//! cut. `octal` describes an escape syntax this language does not have.
//! `unicode` selects character classes, where a token's kind comes from the
//! lexer. `size_limit` and `dfa_size_limit` bound a compiled automaton that
//! is never built - the engine simulates the program over the token stream
//! and its memory is a function of the pattern's size, not of a table.

use crate::ast::{EmptyLoop, Greed, Pattern};
use crate::orbit::OrbitGroup;
use crate::parser::ParseError;

/// A pattern's source together with the readings it is to be parsed under.
pub struct PatternBuilder<'s> {
    src: &'s str,
    orbit: Option<OrbitGroup>,
    swap_greed: bool,
    empty: EmptyLoop,
    nest_limit: u32,
}

impl<'s> PatternBuilder<'s> {
    /// A builder over `src` with every reading at its default, so
    /// `PatternBuilder::new(s).build()` is [`crate::parse`].
    #[must_use]
    pub fn new(src: &'s str) -> Self {
        PatternBuilder {
            src,
            orbit: None,
            swap_greed: false,
            empty: EmptyLoop::Thompson,
            nest_limit: crate::parser::NEST_LIMIT,
        }
    }

    /// The pattern text this builder was given.
    ///
    /// The nearest thing to the regex crate's `as_str`, which exists because a
    /// compiled `Regex` is opaque: once built, the source is the only window
    /// into what it matches. A [`Pattern`] is a public tree the caller can walk
    /// and match on, so the question `as_str` answers does not arise for one -
    /// and a builder is the only place in this crate that holds the text after
    /// parsing, so it is the only place the accessor belongs.
    #[must_use]
    pub fn as_str(&self) -> &'s str {
        self.src
    }

    /// Compare every literal in the pattern under case folding, so `"Cat"`
    /// matches `cat` and `CAT`.
    ///
    /// A register-equality atom keeps whichever group its own spelling gave
    /// it: `=case x` is a comparison the author named, and widening it here
    /// would make the pattern mean something other than what was written.
    #[must_use]
    pub fn case_insensitive(mut self, yes: bool) -> Self {
        self.orbit = yes.then_some(OrbitGroup::Case);
        self
    }

    /// Compare every literal in the pattern under `group`.
    ///
    /// [`Self::case_insensitive`] is this with [`OrbitGroup::Case`], and it is
    /// the only rung the regex crate has a name for. The others have no
    /// counterpart there because a regular expression compares bytes and
    /// these are equivalences over tokens:
    ///
    /// - [`OrbitGroup::Notation`] folds case and notation together, so
    ///   `theta`, `\theta` and the Greek letter are one token.
    /// - [`OrbitGroup::Shape`] compares a token's consonant-vowel-digit
    ///   shape, so `"cat"` matches `dog` and `bat`. A whole pattern under
    ///   this rung is a structural search: find anything shaped like this,
    ///   whatever it says.
    /// - [`OrbitGroup::E8`] compares the token's eight-channel profile
    ///   quotiented by the E8 reflection group, which is the one rung whose
    ///   equivalence is not a string relation at all.
    ///
    /// A register-equality atom keeps whichever group its own spelling gave
    /// it, at every rung: `=shape x` is a comparison the author named.
    #[must_use]
    pub fn orbit(mut self, group: OrbitGroup) -> Self {
        self.orbit = Some(group);
        self
    }

    /// Swap every quantifier's preference, so `*` prefers the shortest match
    /// and `*?` the longest.
    #[must_use]
    pub fn swap_greed(mut self, yes: bool) -> Self {
        self.swap_greed = yes;
        self
    }

    /// Refuse a pattern nesting deeper than `limit` groups.
    #[must_use]
    pub fn nest_limit(mut self, limit: u32) -> Self {
        self.nest_limit = limit;
        self
    }

    /// Which reading a repetition whose body can match nothing takes.
    #[must_use]
    pub fn empty_loop(mut self, empty: EmptyLoop) -> Self {
        self.empty = empty;
        self
    }

    /// The pattern, or the error that says why the source is not one.
    ///
    /// # Errors
    ///
    /// A malformed pattern, or one nesting deeper than the limit.
    pub fn build(self) -> Result<Pattern, ParseError> {
        let (src, _) = crate::parser::split_empty_loop(self.src)?;
        let shapes = crate::custom::ShapeSet::new();
        let mut pat = crate::parser::parse_with_shapes_to_depth(src, &shapes, self.nest_limit)?;
        if let Some(group) = self.orbit {
            crate::parser::set_orbit(&mut pat, group);
        }
        if self.swap_greed {
            swap_greed(&mut pat);
        }
        Ok(pat)
    }

    /// The empty-loop reading a scan of this pattern should take: the one the
    /// source named with `(?empty:...)`, or the one this builder was given.
    ///
    /// The source wins because it is part of the pattern, and a pattern that
    /// says how its own empty loops read is making a claim about what it
    /// means rather than about how some caller wants it run.
    ///
    /// # Errors
    ///
    /// A malformed `(?empty:...)` directive.
    pub fn reading(&self) -> Result<EmptyLoop, ParseError> {
        let (_, named) = crate::parser::split_empty_loop(self.src)?;
        if self.src.trim_start().starts_with("(?empty:") {
            return Ok(named);
        }
        Ok(self.empty)
    }
}

/// Flip the preference of every quantifier in `pat`.
fn swap_greed(pat: &mut Pattern) {
    let flip = |g: &mut Greed| {
        *g = match *g {
            Greed::Greedy => Greed::Lazy,
            Greed::Lazy => Greed::Greedy,
        };
    };
    match pat {
        Pattern::Star(p, g) | Pattern::Plus(p, g) | Pattern::Opt(p, g) => {
            flip(g);
            swap_greed(p);
        }
        Pattern::Repeat(p, _, _, g) => {
            flip(g);
            swap_greed(p);
        }
        Pattern::Bind(_, _, p)
        | Pattern::Balanced(_, p)
        | Pattern::Field(_, p)
        | Pattern::Atomic(p)
        | Pattern::Assert(p, _, _) => swap_greed(p),
        Pattern::Concat(v) | Pattern::Alt(v, _) => {
            for p in v {
                swap_greed(p);
            }
        }
        Pattern::Empty
        | Pattern::Atom(_)
        | Pattern::Within(..)
        | Pattern::Guard(..)
        | Pattern::Anchor(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_builder_at_its_defaults_is_the_ordinary_parse() {
        for src in ["\"alpha\"", "\\W \"=\" \\N", "(\"a\" | \"b\")*", "\\W:x \"=\" =x"] {
            let built = PatternBuilder::new(src).build().expect("builds");
            let plain = crate::parse(src).expect("parses");
            assert_eq!(built, plain, "{src}");
        }
    }

    #[test]
    fn the_source_names_the_empty_loop_reading_and_the_builder_defers_to_it() {
        // A pattern that says how its own empty loops read is making a claim
        // about what it means; a builder default is a caller's preference,
        // and the claim wins.
        let named = PatternBuilder::new("(?empty:perl)\\W*").empty_loop(EmptyLoop::Thompson);
        assert_eq!(named.reading().expect("the directive is well formed"), EmptyLoop::Perl);
        named.build().expect("a source with a directive still builds");

        let unnamed = PatternBuilder::new("\\W*").empty_loop(EmptyLoop::Perl);
        assert_eq!(unnamed.reading().expect("no directive to malform"), EmptyLoop::Perl);
    }

    #[test]
    fn case_folding_makes_a_literal_match_the_other_spellings() {
        let input = b"the Cat sat on the CAT and the cat" as &[u8];
        let folded = PatternBuilder::new("\"cat\"").case_insensitive(true).build().expect("builds");
        let exact = PatternBuilder::new("\"cat\"").build().expect("builds");
        assert_eq!(crate::scan(&folded, input).len(), 3, "Cat, CAT and cat");
        assert_eq!(crate::scan(&exact, input).len(), 1, "only the exact one");
    }

    #[test]
    fn a_whole_pattern_under_the_shape_orbit_is_a_structural_search() {
        // The rung with no regular-expression counterpart: the literal stops
        // asking for its own characters and asks for its consonant-vowel
        // shape, so the pattern finds anything built the same way.
        let input = b"the cat and the dog and the bat and the a1 thing" as &[u8];
        let shaped = PatternBuilder::new("\"cat\"").orbit(OrbitGroup::Shape).build().expect("builds");
        let exact = PatternBuilder::new("\"cat\"").build().expect("builds");
        let found = crate::scan(&shaped, input);
        assert!(found.len() > crate::scan(&exact, input).len(), "the shape matches more than one word");
        for s in &found {
            let word = &input[s.range()];
            assert_eq!(word.len(), 3, "every match is three letters like cat: {:?}", word);
        }
    }

    #[test]
    fn case_folding_leaves_a_named_register_comparison_alone() {
        // `=x` with no group named compares exactly; folding the pattern must
        // not widen a comparison the author wrote, or `\W:x "=" =x` would
        // start matching `A = a`.
        let folded = PatternBuilder::new("\\W:x \"=\" =x")
            .case_insensitive(true)
            .build()
            .expect("builds");
        assert!(crate::scan(&folded, b"alpha = alpha").len() == 1, "the same word still matches");
        assert!(crate::scan(&folded, b"Alpha = alpha").is_empty(), "a differing case does not");
    }

    #[test]
    fn swapping_greed_turns_the_longest_preference_into_the_shortest() {
        let input = b"a b c d ;" as &[u8];
        let greedy = PatternBuilder::new("\\W+").build().expect("builds");
        let lazy = PatternBuilder::new("\\W+").swap_greed(true).build().expect("builds");
        let g = crate::scan(&greedy, input);
        let l = crate::scan(&lazy, input);
        assert_eq!(g.len(), 1, "greedy takes all four words as one match");
        assert_eq!(l.len(), 4, "swapped, each word is its own match");
    }

    #[test]
    fn swapping_greed_twice_is_swapping_it_not_at_all() {
        for src in ["\\W+", "\\W*?", "\\W{2,5}", "(\\W | \\N)+?"] {
            let once = PatternBuilder::new(src).swap_greed(true).build().expect("builds");
            let mut twice = once.clone();
            super::swap_greed(&mut twice);
            assert_eq!(twice, crate::parse(src).expect("parses"), "{src}");
        }
    }

    #[test]
    fn a_pattern_nested_past_the_limit_is_an_error_and_not_a_crash() {
        // The case the limit exists for: without it this recurses until the
        // stack is gone, which a caller cannot catch.
        let deep = format!("{}\"a\"{}", "(".repeat(5_000), ")".repeat(5_000));
        let e = PatternBuilder::new(&deep).build().expect_err("must refuse");
        assert!(e.msg.contains("nests deeper"), "the reason names the depth: {}", e.msg);
        assert!(crate::parse(&deep).is_err(), "the ordinary parse refuses it too");
    }

    #[test]
    fn a_pattern_inside_the_limit_still_parses() {
        // Half the limit, read from the limit rather than written down, since
        // the limit is what the build's stack holds and a debug build holds
        // far less than a release one.
        let depth = (crate::parser::NEST_LIMIT / 2) as usize;
        let ok = format!("{}\"a\"{}", "(".repeat(depth), ")".repeat(depth));
        PatternBuilder::new(&ok).build().expect("half the limit is fine");
        // And a limit a caller chose is the one that applies.
        let e = PatternBuilder::new(&ok).nest_limit(10).build().expect_err("must refuse at ten");
        assert!(e.msg.contains("nests deeper"), "{}", e.msg);
    }
}
