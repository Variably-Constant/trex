//! Orbit canonicalization: the canonical representative of a symmetry orbit.
//!
//! A checkable zero is an invariant under a symmetry group, and the "zero" is
//! reached at the canonical orbit representative. This module computes that
//! representative for two orbits:
//!
//! - the notation orbit ([`canon_symbol`]): a Greek glyph and its TeX command
//!   name are the same symbol, so `\theta`, the glyph, and `theta` collapse to
//!   one entity - the orbit under the choice of surface notation;
//! - the register-renaming orbit ([`rename_invariant_sig`]): two machine-code
//!   blocks that differ only in which registers they use share a signature -
//!   the orbit under permuting the register names.

/// Greek letters as `(glyph, name)`, lowercase. Uppercase glyphs fold to the
/// lowercase name, so the notation orbit collapses case as well as encoding.
const GREEK: &[(&str, &str)] = &[
    ("\u{03B1}", "alpha"),
    ("\u{03B2}", "beta"),
    ("\u{03B3}", "gamma"),
    ("\u{03B4}", "delta"),
    ("\u{03B5}", "epsilon"),
    ("\u{03B6}", "zeta"),
    ("\u{03B7}", "eta"),
    ("\u{03B8}", "theta"),
    ("\u{03B9}", "iota"),
    ("\u{03BA}", "kappa"),
    ("\u{03BB}", "lambda"),
    ("\u{03BC}", "mu"),
    ("\u{03BD}", "nu"),
    ("\u{03BE}", "xi"),
    ("\u{03C0}", "pi"),
    ("\u{03C1}", "rho"),
    ("\u{03C3}", "sigma"),
    ("\u{03C4}", "tau"),
    ("\u{03C6}", "phi"),
    ("\u{03C7}", "chi"),
    ("\u{03C8}", "psi"),
    ("\u{03C9}", "omega"),
];

/// Canonicalize a symbol to its notation-orbit representative: a Greek glyph
/// (`\u{03B8}`), its TeX command (`\theta`), and the bare name (`theta`) all
/// map to one key. A leading backslash is stripped and case is folded.
#[must_use]
pub fn canon_symbol(s: &str) -> String {
    // A glyph, either case, is keyed by its name.
    let lower = s.to_lowercase();
    for (g, n) in GREEK {
        if *g == s || *g == lower {
            return (*n).to_string();
        }
    }
    // For a command or bare name the canonical form is the name with the
    // leading backslash removed and case folded; a Greek name is already it.
    s.trim_start_matches('\\').to_lowercase()
}

/// A register-renaming-invariant signature of a `(read, write)` sequence: each
/// distinct register is relabeled by first-occurrence order, then the relabeled
/// sequence is hashed. Two blocks that differ only in which registers they use
/// (a renaming) produce the same signature - the orbit under register
/// permutation. A slot value of 32 (or any value >= 32) means "no register".
#[must_use]
pub fn rename_invariant_sig(ops: &[(u8, u8)]) -> u64 {
    let mut label = [255u8; 64];
    let mut next = 0u8;
    let mut relabel = |r: u8| -> u8 {
        if r >= 32 {
            return 255; // none
        }
        let idx = r as usize;
        if label[idx] == 255 {
            label[idx] = next;
            next += 1;
        }
        label[idx]
    };
    // FNV-1a over the relabeled (read, write) stream.
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut feed = |b: u8| {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100_0000_01b3);
    };
    for &(r, w) in ops {
        feed(relabel(r));
        feed(relabel(w));
        feed(0xFF);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greek_glyph_and_command_canonicalize_together() {
        assert_eq!(canon_symbol("\u{03B8}"), "theta");
        assert_eq!(canon_symbol("\\theta"), "theta");
        assert_eq!(canon_symbol("theta"), "theta");
        assert_eq!(canon_symbol("\u{0398}"), "theta", "uppercase Theta folds too");
        assert_eq!(canon_symbol("Vec"), "vec", "non-Greek just folds case");
    }

    #[test]
    fn register_renaming_is_invariant() {
        // Block 1 uses regs 0,1; block 2 uses regs 3,7 in the same pattern.
        let b1 = [(0u8, 1u8), (1, 0), (0, 32)];
        let b2 = [(3u8, 7u8), (7, 3), (3, 32)];
        assert_eq!(rename_invariant_sig(&b1), rename_invariant_sig(&b2), "a renaming is invariant");
        // A genuinely different pattern differs.
        let b3 = [(0u8, 1u8), (0, 1), (0, 32)];
        assert_ne!(rename_invariant_sig(&b1), rename_invariant_sig(&b3));
    }
}
