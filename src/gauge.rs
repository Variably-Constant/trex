//! The gauge reading - the meaning left invariant when bound names change.
//!
//! Renaming a bound variable changes nothing: `[ x ( x ) ]` and `[ y ( y ) ]`
//! are the same term. That freedom is a gauge symmetry - the name of a bound
//! variable is pure gauge, carrying no physical content. The [orbit](crate::orbit)
//! axis quotients by surface symmetries (case, notation, shape); this quotients
//! by the deepest one, alpha-equivalence, which needs the binding structure, not
//! just the surface.
//!
//! Gauge-fixing produces a canonical representative. Each binder becomes an
//! anonymous `#`; each bound use becomes `^k`, the de Bruijn index of its binder
//! (how many enclosing `[` scopes up it sits) - name-free and position-free. A
//! free variable is kept literal: it is not gauge, it is a reference to something
//! outside, physical content the canonical form must preserve. Two terms are
//! alpha-equivalent exactly when their gauge-fixed forms match.
//!
//! `[ x ( x ) ]` and `[ y ( y ) ]` both fix to `[ # ( ^0 ) ]`; `[ x ( y ) ]`
//! with `y` free fixes to `[ # ( y ) ]`, distinct - the free reference survives.

use crate::lexer::lex;
use crate::token::{BracketKind, Token, TokenKind};

/// The de Bruijn index of the nearest enclosing binder whose name matches, or
/// `None` when the name is free. Index 0 is the innermost scope.
fn de_bruijn(stack: &[Option<Vec<u8>>], name: &[u8]) -> Option<usize> {
    stack.iter().rev().position(|b| b.as_deref() == Some(name))
}

/// Gauge-fix an already-lexed stream to its alpha-equivalence canonical form.
#[must_use]
pub fn canonicalize(toks: &[Token], bytes: &[u8]) -> String {
    let mut stack: Vec<Option<Vec<u8>>> = Vec::new();
    let mut expecting = false;
    let mut out: Vec<String> = Vec::new();
    for t in toks {
        if !t.is_significant() {
            continue;
        }
        let piece = match t.kind {
            TokenKind::Open(BracketKind::Square) => {
                stack.push(None);
                expecting = true;
                "[".to_string()
            }
            TokenKind::Close(BracketKind::Square) => {
                stack.pop();
                "]".to_string()
            }
            TokenKind::Word => {
                let name = &bytes[t.span()];
                if expecting {
                    // The scope's binder: pure gauge, anonymised.
                    if let Some(slot) = stack.last_mut() {
                        *slot = Some(name.to_vec());
                    }
                    expecting = false;
                    "#".to_string()
                } else if let Some(k) = de_bruijn(&stack, name) {
                    // A bound use: its binder by de Bruijn index, name-free.
                    format!("^{k}")
                } else {
                    // Free: physical, kept literal.
                    String::from_utf8_lossy(name).into_owned()
                }
            }
            // Every other significant token - `(`, `)`, `{`, `}`, punctuation -
            // is structural and copied through unchanged.
            _ => String::from_utf8_lossy(&bytes[t.span()]).into_owned(),
        };
        out.push(piece);
    }
    out.join(" ")
}

/// Gauge-fix directly from bytes.
#[must_use]
pub fn canonicalize_bytes(bytes: &[u8]) -> String {
    canonicalize(&lex(bytes), bytes)
}

/// Whether two streams are alpha-equivalent: the same up to renaming bound
/// variables. Free variables must match by name.
#[must_use]
pub fn alpha_equivalent(a: &[u8], b: &[u8]) -> bool {
    canonicalize_bytes(a) == canonicalize_bytes(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renaming_a_bound_variable_is_gauge() {
        assert!(alpha_equivalent(b"[ x ( x ) ]", b"[ y ( y ) ]"));
        assert_eq!(canonicalize_bytes(b"[ x ( x ) ]"), "[ # ( ^0 ) ]");
    }

    #[test]
    fn a_free_variable_is_physical() {
        // y is free in the second - the canonical forms differ, and swapping the
        // free name changes meaning.
        assert!(!alpha_equivalent(b"[ x ( x ) ]", b"[ x ( y ) ]"));
        assert_eq!(canonicalize_bytes(b"[ x ( y ) ]"), "[ # ( y ) ]");
        assert!(!alpha_equivalent(b"[ x ( y ) ]", b"[ x ( z ) ]"));
    }

    #[test]
    fn shadowing_resolves_to_the_nearest_binder() {
        // The inner use binds to the inner scope (de Bruijn 0), so renaming
        // either bound name leaves the canonical form fixed.
        assert!(alpha_equivalent(b"[ a [ b ( b ) ] ]", b"[ p [ q ( q ) ] ]"));
        assert_eq!(canonicalize_bytes(b"[ a [ b ( b ) ] ]"), "[ # [ # ( ^0 ) ] ]");
    }

    #[test]
    fn a_deeper_reference_has_a_higher_index() {
        // The use binds to the OUTER scope across an inner one: de Bruijn 1.
        assert_eq!(canonicalize_bytes(b"[ a [ b ( a ) ] ]"), "[ # [ # ( ^1 ) ] ]");
    }
}
