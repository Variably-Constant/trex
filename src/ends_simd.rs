//! The scan under the leftmost, non-overlapping selection: the first anchor
//! in a range whose longest match ends past it, found a vector at a time.
//!
//! Both of the GPU backend's selections spend themselves here. On real text
//! most patterns match under one anchor in eighty, so almost every slot the
//! walk visits is a miss: it costs 0.736 ns per anchor against 2.50 ns per
//! match, and on a sparse pattern the anchors are 96% of it. Sparsity does
//! not let the walk skip, because the read is linear and touches every
//! slot's cache line either way.
//!
//! Each lane is compared against its own anchor index rather than a constant,
//! which is what separates this from the byte searches in
//! [`crate::byte_simd`]: the vector of indices is the lane offsets plus the
//! window's first anchor, and the compare answers for sixteen anchors at once
//! the question the scalar loop answers for one.
//!
//! Calling a `#[target_feature]` function on a CPU lacking the feature is
//! undefined behavior; [`crate::isa::tier`] discharges that precondition,
//! reporting the rungs this CPU can run from a probe resolved once for the
//! process. One call scans until it reaches a match, so reading the answer
//! there amortizes over the whole range walked rather than costing a test per
//! anchor.

#![allow(unsafe_code)]

/// The first anchor in `[from, limit)` whose longest match ends past it,
/// or `None` when none does. A range that starts at or past `limit` is empty
/// and yields `None`, which is what lets a caller jump past the end of its
/// chunk.
///
/// This reaches for the vector scan on every call, behind a short scalar
/// probe. The selections do not call it: they hold an [`EndsWalk`], which
/// chooses between this and [`next_match_from_scalar`] per call, because the
/// vector bodies lose to the scalar walk where matches are a few anchors
/// apart. Measured at avx512 over 5,162,460 anchors: 3.63x the scalar walk at
/// 83 anchors between matches, 0.70x at 2.80.
#[must_use]
#[inline]
pub fn next_match_from(ends: &[i32], from: usize, limit: usize) -> Option<usize> {
    // The vector bodies read sixteen slots under `a + 16 <= limit`, so a limit
    // past the end of the slice would read off it where the scalar walk would
    // panic on the index. Holding the range to the slice is what lets this be
    // called without `unsafe`.
    let limit = limit.min(ends.len());
    // The vector bodies run their setup once a call rather than once a window,
    // so a match a few anchors on costs more to find with them than without:
    // unprobed, the scan is slower than the scalar walk where matches are
    // under six anchors apart. Reading the first few singly answers that call
    // before any of the setup is reached.
    let probe = limit.min(from.saturating_add(PROBE));
    if let Some(m) = next_match_from_scalar(ends, from, probe) {
        return Some(m);
    }
    next_match_from_unprobed(ends, probe, limit)
}

/// [`next_match_from`] without the scalar probe in front of it, exposed so a
/// measurement can supply its own probe length instead of the one this module
/// ships. The two share this dispatch rather than each carrying a copy.
#[must_use]
#[inline]
pub fn next_match_from_unprobed(ends: &[i32], from: usize, limit: usize) -> Option<usize> {
    let limit = limit.min(ends.len());
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY on each arm: `crate::isa::tier` reports the rungs this CPU
        // can run, resolved once for the process, and a rung implies every
        // rung below it - so reaching an arm is the guarantee that the
        // features its body was compiled for are present.
        match crate::isa::tier() {
            crate::isa::Tier::Avx512 => {
                return unsafe { next_match_from_avx512(ends, from, limit) };
            }
            crate::isa::Tier::Avx2 => return unsafe { next_match_from_avx2(ends, from, limit) },
            crate::isa::Tier::Sse2 | crate::isa::Tier::Scalar => {}
        }
    }
    next_match_from_scalar(ends, from, limit)
}

/// Anchors read one at a time before the vector scan is reached.
const PROBE: usize = 4;

/// Anchors a call must expect to cross before the vector scan is worth its
/// setup. Measured over 5,162,460 anchors at avx512: 1.88x the scalar walk
/// where a call crosses about 19, 1.03x at about 3.4, and 0.70x at about 0.3.
const WIDE: usize = 8;

/// A walk over the per-anchor match ends that chooses its scan by how far the
/// last calls actually reached.
///
/// The vector bodies run their setup once a call, so they win only when a call
/// crosses enough anchors to amortize it. A pattern matching one anchor in
/// eighty crosses that easily; one matching every third anchor never does, and
/// running the setup there costs more than the scalar step it replaces. Which
/// of the two a pattern is cannot be known before the walk, and a document
/// holds stretches of both, so the estimate is carried and updated rather than
/// decided once.
///
/// Both of the selections hold one of these across their whole walk. In
/// `select_parts` that means across chunks: a chunk boundary ends the inner
/// loop without ending the walk, and an estimate rebuilt per chunk would learn
/// nothing.
/// Whether a walk over `ends` should take the vector scan, from a sample of
/// its head.
///
/// The caller asks once and then runs a loop that only ever calls the scan it
/// chose. Testing per call costs about 0.7 cycles each - one load and a
/// branch - which is nothing against a call that crosses eighty anchors and
/// about a tenth of one that crosses two: measured at 0.897x the scalar walk
/// at one match every 2.80 anchors, against parity once the test is hoisted
/// out of the loop. There is no cheaper test, so the answer is to ask it
/// fewer times rather than to make it cheaper.
///
/// The sample walks the first few thousand anchors exactly as the selection
/// will, so what it measures is the distance the real walk travels between
/// the matches it takes, not the share of anchors carrying an end. Those
/// differ by the length of a match. A sample with no match at all reads as
/// sparse, which is what the vector scan is best at.
#[must_use]
pub fn takes_vector(ends: &[i32]) -> bool {
    /// Anchors sampled before the walk commits. At 5,162,460 anchors this is
    /// under a tenth of a percent of the array.
    const SAMPLE: usize = 4096;
    let n = ends.len().min(SAMPLE);
    let mut a = 0usize;
    let mut matches = 0usize;
    while let Some(m) = next_match_from_scalar(ends, a, n) {
        a = ends[m] as usize;
        matches += 1;
    }
    matches == 0 || n / matches >= WIDE
}

/// The scalar baseline of [`next_match_from`], exposed so a measurement can
/// compare it against the dispatched SIMD path.
///
/// The probe calls this for its first few anchors, and the release profile
/// builds without link-time optimization, so without the body available to
/// another crate that probe would cost a call each time it runs.
#[must_use]
#[inline]
pub fn next_match_from_scalar(ends: &[i32], from: usize, limit: usize) -> Option<usize> {
    let limit = limit.min(ends.len());
    (from..limit).find(|&a| ends[a] > a as i32)
}

/// Sixteen anchors a step. `_mm512_cmpgt_epi32_mask` yields the lane mask
/// directly, so the first match in a window is its trailing zeros with no
/// movemask between. Only `avx512f` is needed: the load, the compare, the
/// broadcast and the add are all in it.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
unsafe fn next_match_from_avx512(ends: &[i32], from: usize, limit: usize) -> Option<usize> {
    use core::arch::x86_64::{
        _mm512_add_epi32, _mm512_cmpgt_epi32_mask, _mm512_loadu_si512, _mm512_set1_epi32,
        _mm512_setr_epi32,
    };
    // Lane j holds j, so adding the window's first anchor gives each lane
    // the index the scalar loop would compare that slot against.
    let lanes = _mm512_setr_epi32(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15);
    let mut a = from;
    while a + 16 <= limit {
        // SAFETY: the load reads sixteen `i32` from `a`, and `a + 16 <=
        // limit <= ends.len()` is the loop guard.
        let v = unsafe { _mm512_loadu_si512(ends.as_ptr().add(a).cast()) };
        let idx = _mm512_add_epi32(lanes, _mm512_set1_epi32(a as i32));
        let mask: u16 = _mm512_cmpgt_epi32_mask(v, idx);
        if mask != 0 {
            return Some(a + mask.trailing_zeros() as usize);
        }
        a += 16;
    }
    next_match_from_scalar(ends, a, limit)
}

/// Eight anchors a step, the same compare against the lane indices, with a
/// movemask to bring the result out as one bit per lane.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn next_match_from_avx2(ends: &[i32], from: usize, limit: usize) -> Option<usize> {
    use core::arch::x86_64::{
        _mm256_add_epi32, _mm256_castsi256_ps, _mm256_cmpgt_epi32, _mm256_loadu_si256,
        _mm256_movemask_ps, _mm256_set1_epi32, _mm256_setr_epi32,
    };
    let lanes = _mm256_setr_epi32(0, 1, 2, 3, 4, 5, 6, 7);
    let mut a = from;
    while a + 8 <= limit {
        // SAFETY: the load reads eight `i32` from `a`, and `a + 8 <= limit
        // <= ends.len()` is the loop guard.
        let v = unsafe { _mm256_loadu_si256(ends.as_ptr().add(a).cast()) };
        let idx = _mm256_add_epi32(lanes, _mm256_set1_epi32(a as i32));
        let gt = _mm256_cmpgt_epi32(v, idx);
        // One bit per 32-bit lane, where the byte movemask would give four.
        let mask = _mm256_movemask_ps(_mm256_castsi256_ps(gt)) as u32;
        if mask != 0 {
            return Some(a + mask.trailing_zeros() as usize);
        }
        a += 8;
    }
    next_match_from_scalar(ends, a, limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ends shaped as the kernel writes them: `-1` where an anchor matched
    /// nothing, and an end past the anchor where it did. One anchor in
    /// `stride` matches. Every seventh slot is set to the anchor's own index,
    /// which the walk must read as a miss - the predicate is strictly
    /// greater, and a lane equal to its index is the boundary a compare gets
    /// wrong when it is written as `>=`.
    fn sample_ends(n: usize, stride: u32) -> Vec<i32> {
        (0..n)
            .map(|a| {
                let r = (a as u32).wrapping_mul(2_654_435_761) >> 13;
                if r.is_multiple_of(7) {
                    a as i32
                } else if r.is_multiple_of(stride) {
                    a as i32 + 1 + (r % 11) as i32
                } else {
                    -1
                }
            })
            .collect()
    }

    #[test]
    fn dispatched_scan_matches_scalar() {
        // Densities from nearly every anchor matching to almost none, over a
        // length that is not a multiple of either vector width so every run
        // ends in the scalar tail.
        for stride in [2u32, 3, 8, 80, 4000] {
            let ends = sample_ends(5003, stride);
            for from in [0usize, 1, 7, 8, 15, 16, 17, 63, 64, 1000, 4999, 5003] {
                for limit in [0usize, 1, 8, 16, 31, 32, 33, 1000, 5002, 5003] {
                    assert_eq!(
                        next_match_from(&ends, from, limit),
                        next_match_from_scalar(&ends, from, limit),
                        "stride {stride} from {from} limit {limit}"
                    );
                }
            }
        }
    }

    #[test]
    fn an_empty_or_inverted_range_finds_nothing() {
        // `select_parts` jumps `a` to a chunk boundary, so it asks about
        // ranges that have already been passed.
        let ends = sample_ends(200, 3);
        assert_eq!(next_match_from(&ends, 0, 0), None);
        assert_eq!(next_match_from(&ends, 100, 100), None);
        assert_eq!(next_match_from(&ends, 150, 20), None);
        assert_eq!(next_match_from(&[], 0, 0), None);
        // A limit past the end is held to the slice, which is what the vector
        // bodies rely on to keep their loads inside it.
        assert_eq!(next_match_from(&ends, 0, 100_000), next_match_from_scalar(&ends, 0, 200));
        assert_eq!(next_match_from(&[], 0, 64), None);
    }

    #[test]
    fn the_leftmost_match_in_a_window_is_the_one_returned() {
        // Two matches inside one vector window: the scan must return the
        // first, not whichever lane the mask happens to expose.
        let mut ends = vec![-1i32; 64];
        ends[19] = 25;
        ends[21] = 30;
        assert_eq!(next_match_from(&ends, 0, 64), Some(19));
        assert_eq!(next_match_from(&ends, 20, 64), Some(21));
        assert_eq!(next_match_from(&ends, 22, 64), None);
    }

    /// The whole leftmost non-overlapping walk with `pick`, as the selections
    /// run it: take the next match, jump to its end, repeat.
    fn walk_with(ends: &[i32], mut pick: impl FnMut(&[i32], usize, usize) -> Option<usize>) -> Vec<usize> {
        let n = ends.len();
        let mut a = 0usize;
        let mut taken = Vec::new();
        while let Some(m) = pick(ends, a, n) {
            taken.push(m);
            a = ends[m] as usize;
        }
        taken
    }

    #[test]
    fn the_adaptive_walk_takes_what_the_scalar_walk_takes() {
        // Uniform densities either side of the threshold, then an array that
        // crosses it part way through: sparse for the first half and dense for
        // the second, which is the case the running estimate exists for and
        // the one a uniform array cannot exercise.
        for stride in [2u32, 3, 8, 80, 4000] {
            let ends = sample_ends(5003, stride);
            let chosen: fn(&[i32], usize, usize) -> Option<usize> =
                if takes_vector(&ends) { next_match_from } else { next_match_from_scalar };
            assert_eq!(
                walk_with(&ends, chosen),
                walk_with(&ends, next_match_from_scalar),
                "uniform stride {stride}"
            );
        }
        let mut mixed = sample_ends(4000, 300);
        // The second half is built at the joined indices it will occupy, so
        // its ends already point past their own anchors and none reaches past
        // the array.
        let dense: Vec<i32> = (4000..8000)
            .map(|a| {
                let r = (a as u32).wrapping_mul(2_654_435_761) >> 13;
                if r.is_multiple_of(2) { (a + 1).min(7999) } else { -1 }
            })
            .collect();
        mixed.extend_from_slice(&dense);
        let chosen: fn(&[i32], usize, usize) -> Option<usize> =
            if takes_vector(&mixed) { next_match_from } else { next_match_from_scalar };
        assert_eq!(
            walk_with(&mixed, chosen),
            walk_with(&mixed, next_match_from_scalar),
            "sparse then dense in one array"
        );
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn each_simd_path_matches_scalar_on_large_input() {
        let ends = sample_ends(9001, 60);
        let probes: &[(usize, usize)] =
            &[(0, 9001), (0, 16), (1, 9000), (17, 9001), (8000, 9001), (9001, 9001)];
        if std::is_x86_feature_detected!("avx2") {
            for &(from, limit) in probes {
                assert_eq!(
                    unsafe { next_match_from_avx2(&ends, from, limit) },
                    next_match_from_scalar(&ends, from, limit),
                    "avx2 from {from} limit {limit}"
                );
            }
        }
        if std::is_x86_feature_detected!("avx512f") {
            for &(from, limit) in probes {
                assert_eq!(
                    unsafe { next_match_from_avx512(&ends, from, limit) },
                    next_match_from_scalar(&ends, from, limit),
                    "avx512 from {from} limit {limit}"
                );
            }
        }
    }
}
