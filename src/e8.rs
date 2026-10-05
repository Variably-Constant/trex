//! The E8 root lattice and its Weyl group - the symmetry substrate under the deepest
//! [`crate::orbit`] rung.
//!
//! # Why E8 and not any 8-dimensional lattice
//!
//! E8 is the unique even self-dual lattice in dimension 8, its 240 roots all have the same
//! length, and its reflection group `W(E8)` (order 696 729 600) acts transitively on those roots.
//! That transitivity is the whole point for tokenization: it means "is a root" is a symmetry
//! CLASS rather than a list, so a feature vector's orbit representative is a statement about its
//! structure and not about which axis happened to carry which number.
//!
//! # Coordinates
//!
//! The standard construction: a point is in E8 exactly when its coordinates are either all
//! integers or all half-odd-integers, and their sum is an even integer. Vectors are stored
//! doubled (`[i64; 8]` holding `2x`), so half-integers are exact and every operation stays in
//! integer arithmetic - there is no floating point anywhere in this module, and therefore no
//! rounding question about whether two vectors are the same point.
//!
//! # Canonicalization
//!
//! The orbit representative is the vector's image in the fundamental Weyl chamber: the unique
//! point of its orbit whose inner product with every simple root is non-negative. It is reached
//! by repeatedly reflecting in any simple root the vector is negative against
//! (`v -> v - <v, a> a`, valid because every root has norm 2). Each reflection flips a negative
//! pairing positive and strictly reduces how many positive roots the vector is negative against,
//! so the walk terminates - the standard "reflect until dominant" argument.
//!
//! # What is checkable here, and is checked
//!
//! - All 240 roots are in one orbit, so they must all canonicalize to a single vector. That is a
//!   240-way agreement test no implementation passes by accident.
//! - Canonicalization is idempotent, norm-preserving, and its output is dominant.
//! - A vector and its image under a random word in the reflection generators canonicalize
//!   identically.
//! - The root system itself is verified: exactly 240 vectors, all of norm 2, closed under
//!   negation.

/// A doubled E8 vector: entry `i` holds `2 * x_i`, so half-integer coordinates are exact.
pub type E8Vec = [i64; 8];

/// The doubled inner product. For doubled vectors `a = 2x`, `b = 2y` this returns `4<x, y>`, so
/// Signs and zeros - all the canonicalization needs - are exact and scale-free.
#[must_use]
pub fn dot(a: &E8Vec, b: &E8Vec) -> i64 {
    let mut s = 0i64;
    for i in 0..8 {
        s += a[i] * b[i];
    }
    s
}

/// Squared norm in doubled coordinates: `4|x|^2`. A root therefore reads 8, not 2.
#[must_use]
pub fn norm2(v: &E8Vec) -> i64 {
    dot(v, v)
}

/// Is `v` a point of the E8 lattice?
///
/// All coordinates integral or all half-odd-integral, and the coordinate sum even. In doubled
/// form: every entry even (integer case) or every entry odd (half-integer case), and in both
/// cases `sum(v) % 4 == 0` - the doubled restatement of "sum of the true coordinates is even".
#[must_use]
pub fn in_e8(v: &E8Vec) -> bool {
    let all_even = v.iter().all(|x| x.rem_euclid(2) == 0);
    let all_odd = v.iter().all(|x| x.rem_euclid(2) == 1);
    if !all_even && !all_odd {
        return false;
    }
    v.iter().sum::<i64>().rem_euclid(4) == 0
}

/// The 240 roots of E8, in doubled coordinates (each of doubled norm 8).
///
/// Two families, which is the classical description: 112 of the form `(+-1, +-1, 0^6)` over all
/// axis pairs and signs, and 128 of the form `(+-1/2)^8` with an even number of minus signs.
#[must_use]
pub fn roots() -> Vec<E8Vec> {
    let mut out = Vec::with_capacity(240);
    // (+-1, +-1, 0^6): doubled, that is +-2 in two positions.
    for i in 0..8 {
        for j in (i + 1)..8 {
            for &si in &[2i64, -2] {
                for &sj in &[2i64, -2] {
                    let mut v = [0i64; 8];
                    v[i] = si;
                    v[j] = sj;
                    out.push(v);
                }
            }
        }
    }
    // (+-1/2)^8 with an even number of minus signs: doubled, that is +-1 in every position.
    for mask in 0u32..256 {
        if mask.count_ones() % 2 != 0 {
            continue;
        }
        let mut v = [0i64; 8];
        for (i, e) in v.iter_mut().enumerate() {
            *e = if mask & (1 << i) == 0 { 1 } else { -1 };
        }
        out.push(v);
    }
    out
}

/// The eight simple roots, Bourbaki ordering, in doubled coordinates.
///
/// These generate `W(E8)`; a vector is dominant exactly when it has non-negative inner product
/// with all of them, which is the termination condition [`canonicalize`] walks toward.
#[must_use]
pub fn simple_roots() -> [E8Vec; 8] {
    [
        // a1 = 1/2(e1 - e2 - e3 - e4 - e5 - e6 - e7 + e8)
        [1, -1, -1, -1, -1, -1, -1, 1],
        // a2 = e1 + e2
        [2, 2, 0, 0, 0, 0, 0, 0],
        // a3 = e2 - e1
        [-2, 2, 0, 0, 0, 0, 0, 0],
        // a4 = e3 - e2
        [0, -2, 2, 0, 0, 0, 0, 0],
        // a5 = e4 - e3
        [0, 0, -2, 2, 0, 0, 0, 0],
        // a6 = e5 - e4
        [0, 0, 0, -2, 2, 0, 0, 0],
        // a7 = e6 - e5
        [0, 0, 0, 0, -2, 2, 0, 0],
        // a8 = e7 - e6
        [0, 0, 0, 0, 0, -2, 2, 0],
    ]
}

/// Reflect `v` in the hyperplane orthogonal to root `r`: `v - 2<v,r>/<r,r> * r`.
///
/// In doubled coordinates `<r,r>` is 8 for every root, so `2<v,r>/<r,r>` reduces to `<v,r> / 4`.
/// That division is exact for any lattice `v`, and it is ASSERTED rather than assumed: a silent
/// truncation here would corrupt the orbit quietly instead of failing.
#[must_use]
pub fn reflect(v: &E8Vec, r: &E8Vec) -> E8Vec {
    let d = dot(v, r);
    debug_assert_eq!(norm2(r), 8, "reflect expects a root (doubled norm 8), got {}", norm2(r));
    debug_assert_eq!(d.rem_euclid(4), 0, "<v,r> must be divisible by 4 for a lattice v; got {d}");
    let k = d / 4;
    let mut out = *v;
    for i in 0..8 {
        out[i] -= k * r[i];
    }
    out
}

/// Is `v` in the fundamental Weyl chamber (non-negative against every simple root)?
#[must_use]
pub fn is_dominant(v: &E8Vec) -> bool {
    simple_roots().iter().all(|a| dot(v, a) >= 0)
}

/// The canonical representative of `v`'s `W(E8)` orbit: its unique dominant image.
///
/// Reflect in any simple root the vector is negative against, and repeat. A hard iteration cap is
/// kept even though the walk provably terminates, so a malformed input can never hang a caller -
/// and exceeding it panics rather than returning a wrong answer quietly.
#[must_use]
pub fn canonicalize(v: &E8Vec) -> E8Vec {
    let simples = simple_roots();
    let mut cur = *v;
    // E8 has 120 positive roots; a reflect-until-dominant walk cannot need more sign flips than
    // that, and 4x is generous headroom for the scan order.
    const MAX_STEPS: usize = 480;
    for _ in 0..MAX_STEPS {
        let mut moved = false;
        for a in &simples {
            if dot(&cur, a) < 0 {
                cur = reflect(&cur, a);
                moved = true;
            }
        }
        if !moved {
            return cur;
        }
    }
    panic!("E8 canonicalize did not reach the dominant chamber in {MAX_STEPS} steps for {v:?}");
}

/// Are `a` and `b` in the same `W(E8)` orbit?
#[must_use]
pub fn same_orbit(a: &E8Vec, b: &E8Vec) -> bool {
    canonicalize(a) == canonicalize(b)
}

/// Snap an arbitrary 8-vector (given doubled, so callers keep integer arithmetic) to a nearby E8
/// lattice point.
///
/// Both cosets are tried - round to the integer lattice and round to the all-half-integer copy -
/// and the nearer is kept. Within a coset the even-sum condition is repaired by moving the
/// coordinate that was rounded hardest, which is the cheapest way back onto the sublattice.
///
/// This is a NEAR-point decoder, not a proven-nearest one: it is used to place feature vectors
/// onto the lattice before quotienting, where "on the lattice, deterministically, close by" is
/// the requirement. Anything needing provable nearest-point should say so and use a full decoder.
#[must_use]
pub fn snap(v: &E8Vec) -> E8Vec {
    let a = snap_coset(v, 0);
    let b = snap_coset(v, 1);
    if dist2(v, &a) <= dist2(v, &b) { a } else { b }
}

/// Squared distance in doubled coordinates.
fn dist2(a: &E8Vec, b: &E8Vec) -> i64 {
    let mut s = 0i64;
    for i in 0..8 {
        let d = a[i] - b[i];
        s += d * d;
    }
    s
}

/// Round onto the integer coset (`parity = 0`) or the all-half-integer coset (`parity = 1`), then
/// repair the even-sum condition by flipping the worst-rounded coordinate.
fn snap_coset(v: &E8Vec, parity: i64) -> E8Vec {
    let mut out = [0i64; 8];
    let (mut worst_i, mut worst_cost, mut worst_dir) = (0usize, -1i64, 0i64);
    for i in 0..8 {
        let x = v[i];
        // In doubled coordinates an integer is an even entry and a half-integer an odd one, so
        // rounding to the coset means rounding to the nearest entry of the right parity.
        let mut r = if x.rem_euclid(2) == parity { x } else { x + 1 };
        if (x - (r - 2)).abs() < (x - r).abs() {
            r -= 2;
        }
        out[i] = r;
        let cost = (x - r).abs();
        if cost > worst_cost {
            worst_cost = cost;
            worst_i = i;
            worst_dir = if x >= r { 2 } else { -2 };
        }
    }
    if out.iter().sum::<i64>().rem_euclid(4) != 0 {
        out[worst_i] += worst_dir;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The root system is what the module claims. Everything downstream rests on it, so it is
    /// checked rather than trusted: 240 distinct vectors, each of doubled norm 8, each a lattice
    /// point, the set closed under negation.
    #[test]
    fn the_root_system_is_240_vectors_of_norm_2_closed_under_negation() {
        let rs = roots();
        assert_eq!(rs.len(), 240, "E8 has exactly 240 roots");
        let set: std::collections::BTreeSet<E8Vec> = rs.iter().copied().collect();
        assert_eq!(set.len(), 240, "the roots are distinct");
        for r in &rs {
            assert_eq!(norm2(r), 8, "every root has doubled norm 8 (true norm 2): {r:?}");
            assert!(in_e8(r), "every root is a lattice point: {r:?}");
            assert!(set.contains(&r.map(|x| -x)), "the root system is closed under negation: {r:?}");
        }
    }

    /// The simple roots are genuine roots, and the chamber they define is a proper cone.
    #[test]
    fn simple_roots_are_roots_and_define_a_one_sided_chamber() {
        let set: std::collections::BTreeSet<E8Vec> = roots().into_iter().collect();
        for a in &simple_roots() {
            assert_eq!(norm2(a), 8, "simple root has norm 2: {a:?}");
            assert!(set.contains(a), "simple root is in the root system: {a:?}");
        }
        let dom = canonicalize(&[2, 4, 6, 8, 10, 12, 14, 16]);
        assert!(is_dominant(&dom));
        assert!(!is_dominant(&dom.map(|x| -x)), "the chamber is one-sided, not everything");
    }

    /// The oracle for this module: `W(E8)` is transitive on the 240 roots, so every root must
    /// canonicalize to a single dominant vector. A 240-way agreement is not something a wrong
    /// implementation stumbles into.
    #[test]
    fn all_240_roots_are_one_orbit() {
        let rs = roots();
        let first = canonicalize(&rs[0]);
        assert!(is_dominant(&first), "the representative is dominant: {first:?}");
        for r in &rs {
            let c = canonicalize(r);
            assert_eq!(c, first, "root {r:?} canonicalized to {c:?}, expected the single root orbit {first:?}");
        }
        assert_eq!(norm2(&first), 8, "the orbit did not leave the root shell");
    }

    /// Canonicalization is a projection onto the chamber, and reflections are isometries.
    #[test]
    fn canonicalize_is_idempotent_norm_preserving_and_lands_dominant() {
        let mut s = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s % 21) as i64 - 10
        };
        for _ in 0..200 {
            let mut v = [0i64; 8];
            for e in &mut v {
                *e = next() * 2;
            }
            let v = snap(&v);
            let c = canonicalize(&v);
            assert!(is_dominant(&c), "canonical form is dominant: {v:?} -> {c:?}");
            assert_eq!(canonicalize(&c), c, "canonicalization is idempotent for {v:?}");
            assert_eq!(norm2(&c), norm2(&v), "reflections are isometries - the norm is preserved");
        }
    }

    /// The invariance that makes this a quotient map. Reflections in arbitrary roots are used,
    /// not only the simple ones, so the whole group is exercised rather than the walk's own moves.
    #[test]
    fn a_vector_and_its_weyl_images_share_one_representative() {
        let rs = roots();
        let mut s = 0xDEAD_BEEF_1234_5678u64;
        let mut next = |n: u64| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s % n) as usize
        };
        for _ in 0..100 {
            let mut v = [0i64; 8];
            for e in &mut v {
                *e = (next(11) as i64 - 5) * 2;
            }
            let v = snap(&v);
            assert!(in_e8(&v), "snap produces a lattice point: {v:?}");
            let base = canonicalize(&v);
            let mut w = v;
            for _ in 0..6 {
                w = reflect(&w, &rs[next(240)]);
            }
            assert!(in_e8(&w), "the group preserves the lattice: {w:?}");
            assert_eq!(canonicalize(&w), base, "a Weyl image shares the representative: {v:?} -> {w:?}");
        }
    }

    /// `snap` reaches the lattice from arbitrary input and fixes points already on it.
    #[test]
    fn snap_reaches_the_lattice_and_fixes_lattice_points() {
        for r in roots() {
            assert_eq!(snap(&r), r, "a lattice point is its own snap: {r:?}");
        }
        let mut s = 0x0123_4567_89AB_CDEFu64;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s % 41) as i64 - 20
        };
        for _ in 0..300 {
            let mut v = [0i64; 8];
            for e in &mut v {
                *e = next();
            }
            let p = snap(&v);
            assert!(in_e8(&p), "snap({v:?}) = {p:?} must be a lattice point");
        }
    }

    /// Membership rejects the near-misses a sloppy predicate accepts: mixed parity, and an
    /// all-odd vector whose true coordinate sum is odd. The second is the deep-hole-adjacent case
    /// that an "all odd is fine" rule wrongly admits.
    #[test]
    fn membership_rejects_mixed_parity_and_odd_sums() {
        assert!(in_e8(&[2, 2, 0, 0, 0, 0, 0, 0]), "an integer root is in");
        assert!(in_e8(&[1, 1, 1, 1, 1, 1, 1, 1]), "the all-halves vector with even sum is in");
        assert!(!in_e8(&[1, 2, 0, 0, 0, 0, 0, 0]), "mixed parity is out");
        // (1.5, 0.5 x7): all half-odd-integers, but the true coordinate sum is 5 - odd, so this
        // is not a lattice point. It is at distance 1 from E8, a deep hole, and an "all odd is
        // fine" rule admits it.
        assert!(!in_e8(&[3, 1, 1, 1, 1, 1, 1, 1]), "all-odd with an odd true sum is out");
        // Flipping three signs makes the true sum 2, which is in the lattice, so the rule tests
        // the sum, not merely the parity pattern.
        assert!(in_e8(&[3, 1, 1, 1, -1, -1, -1, 1]), "all-odd with an even true sum is in");
    }
}
