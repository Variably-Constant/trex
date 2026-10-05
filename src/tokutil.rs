//! Substrate token utilities shared across the field axes.
//!
//! Pure token-pattern primitives with no domain knowledge: significant-token lexing, the identity
//! hasher for pre-hashed `u64` key tables, and the language-agnostic identifier-shape classifier.
//! The seam / flow / shape / stress / magnitude axes use these directly; the code-POS modules that
//! also need them re-export from here, so the utilities live in exactly one place.

use crate::lexer::lex;
use crate::token::Token;

/// Significant tokens of `bytes` (whitespace dropped).
#[must_use]
pub fn lex_sig(bytes: &[u8]) -> Vec<Token> {
    lex(bytes).into_iter().filter(Token::is_significant).collect()
}

/// Identity hasher for `u64` key tables - the key already IS the hash, so the `HashMap` does no
/// further mixing.
#[derive(Default)]
pub struct IdHash(u64);
impl std::hash::Hasher for IdHash {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, _: &[u8]) {
        unreachable!("IdHash only takes write_u64")
    }
    #[inline]
    fn write_u64(&mut self, n: u64) {
        self.0 = n;
    }
}

/// The shape of an identifier - a language-agnostic signal: `SCREAM` / `Pascal` / `snake` / `camel`
/// / `short` / `word`. Case is read from each character, so `Επειδή` is `Pascal` as `Whereas` is,
/// and a script with no case reads `short` or `word`; `short` is four characters or fewer.
#[must_use]
pub fn shape(w: &str) -> &'static str {
    let up = w.chars().any(char::is_uppercase);
    let lo = w.chars().any(char::is_lowercase);
    if !lo && (up || w.contains('_')) {
        "SCREAM"
    } else if w.chars().next().is_some_and(char::is_uppercase) && lo {
        "Pascal"
    } else if w.contains('_') {
        "snake"
    } else if up && lo {
        "camel"
    } else if w.chars().count() <= 4 {
        "short"
    } else {
        "word"
    }
}

#[cfg(test)]
mod tests {
    use super::shape;

    #[test]
    fn a_word_reads_the_same_shape_in_any_script() {
        // Words of the Universal Declaration's opening, in English, Greek and
        // Russian: case is a property of the character, not of ASCII.
        for (w, expect) in [("Whereas", "Pascal"), ("Επειδή", "Pascal"), ("Принимая", "Pascal"), ("recognition", "word"), ("αναγνώριση", "word"), ("the", "short"), ("της", "short")] {
            assert_eq!(shape(w), expect, "{w}");
        }
        // A script with no case reads by length, counted in characters: a
        // two-syllable Korean word is short though it is six bytes.
        for (w, expect) in [("인류", "short"), ("في", "short"), ("الإعلان", "word"), ("世界人权宣言", "word")] {
            assert_eq!(shape(w), expect, "{w}");
        }
        for (w, expect) in [("HTML", "SCREAM"), ("snake_case", "snake"), ("camelCase", "camel")] {
            assert_eq!(shape(w), expect, "{w}");
        }
    }
}
