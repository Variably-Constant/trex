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
/// / `short` / `word`.
#[must_use]
pub fn shape(w: &str) -> &'static str {
    let up = w.chars().any(|c| c.is_ascii_uppercase());
    let lo = w.chars().any(|c| c.is_ascii_lowercase());
    if !lo && (up || w.contains('_')) {
        "SCREAM"
    } else if w.chars().next().is_some_and(|c| c.is_ascii_uppercase()) && lo {
        "Pascal"
    } else if w.contains('_') {
        "snake"
    } else if up && lo {
        "camel"
    } else if w.len() <= 4 {
        "short"
    } else {
        "word"
    }
}
