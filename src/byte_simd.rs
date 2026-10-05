//! Hand-rolled SIMD byte search: a memchr-style substring find with
//! runtime CPU-feature dispatch (AVX2 / SSE2 / scalar) and a bit-
//! identical contract - every path returns the same leftmost match
//! position as the scalar baseline. This is the speed layer under the
//! content guard, which searches a window for a literal.
//!
//! The SIMD paths probe the first needle byte 32 (AVX2) or 16 (SSE2)
//! positions at a time, then verify the full needle at each candidate.
//! Calling a `#[target_feature]` function on a CPU lacking the feature
//! is undefined behavior; [`crate::isa::tier`] discharges that
//! precondition, reporting the rungs this CPU can run from a probe
//! resolved once for the process. These searches run per guard literal
//! per position, so the answer is read from there rather than re-tested
//! at each call.
//!
//! The tests below probe the features directly instead. A test that
//! verifies a path is the wrong place to depend on the cache that
//! selects it, and `crate::isa` carries its own test that the two agree.

#![allow(unsafe_code)]

/// The leftmost position at which `needle` occurs in `haystack`, or
/// `None`. An empty needle matches at position 0.
#[must_use]
pub fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > haystack.len() {
        return None;
    }
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY on each arm: `crate::isa::tier` reports the rungs this CPU
        // can run, resolved once for the process, and a rung implies every
        // rung below it - so reaching an arm is the guarantee that the
        // features its body was compiled for are present. A search runs per
        // guard literal per position, which is why the probe is read from
        // there rather than re-tested here.
        match crate::isa::tier() {
            crate::isa::Tier::Avx512 => return unsafe { find_avx512(haystack, needle) },
            crate::isa::Tier::Avx2 => return unsafe { find_avx2(haystack, needle) },
            crate::isa::Tier::Sse2 => return unsafe { find_sse2(haystack, needle) },
            crate::isa::Tier::Scalar => {}
        }
    }
    find_scalar(haystack, needle)
}

/// Whether `needle` occurs anywhere in `haystack`.
#[must_use]
pub fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    find(haystack, needle).is_some()
}

/// Every position at which `needle` occurs in `haystack`, ascending,
/// overlapping occurrences included, searched in one pass or across the cores
/// by which of the two is faster at this size.
///
/// The literal routes walk all of them, which is [`find`] resumed a byte past
/// each hit. An empty needle reports nothing rather than every position: a
/// route asking where a literal is has no use for that answer, and building it
/// would be one entry a byte.
#[must_use]
pub fn find_all(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    if haystack.len() < FIND_ALL_PARALLEL_THRESHOLD {
        crate::trace::rung("find all", "one pass, under the split's size", haystack.len());
        return find_all_one_pass(haystack, needle);
    }
    crate::trace::rung("find all", "split across the cores", haystack.len());
    find_all_across(haystack, needle)
}

/// The size at or above which [`find_all`] splits the search across the cores.
///
/// `benches/find_all_crossover` times the two forms over seven sizes and two
/// needles, on 24 cores:
///
/// | bytes | `let` | `zzzqqq` |
/// |---|---|---|
/// | 128 K | 0.60x | 0.10x |
/// | 512 K | 1.85x | 0.39x |
/// | 2 M | 4.69x | 1.32x |
/// | 7.34 M | 7.31x | 3.34x |
///
/// Two needles because the per-byte cost of the search runs over an order of
/// magnitude between them: `let` opens a quarter of the statements, so nearly
/// every vector hit is verified, and `zzzqqq` occurs nowhere, so the scan runs
/// at its own rate. The split form has a floor of about twenty microseconds
/// whatever the size, which is what the small rows are reading; the rare needle
/// is the one that has to clear it, and two megabytes is the smallest size
/// measured where both do.
///
/// Re-measure with that bench when the per-byte cost of [`find`] moves.
const FIND_ALL_PARALLEL_THRESHOLD: usize = 2 * 1024 * 1024;

/// How much of the haystack [`occurrences`] searches at a time.
///
/// Above [`FIND_ALL_PARALLEL_THRESHOLD`], so a span is still worth splitting,
/// and above the comparison corpus, so an ordinary whole-input walk is one span
/// and costs one dispatch. What the span bounds is the degenerate case: a needle
/// occurring at nearly every position costs eight bytes an occurrence, which
/// over a whole large input is several times the input itself.
const FIND_ALL_SPAN: usize = 8 * 1024 * 1024;

/// Every position at which `needle` occurs in `haystack`, ascending,
/// overlapping occurrences included, a span of the haystack at a time.
///
/// What the literal routes walk. The search is the one part of such a walk that
/// splits, an occurrence depending on its own bytes and nothing else, and this
/// splits it; what a route does with each occurrence is its own and stays
/// serial. Holding one span's occurrences rather than the whole input's is what
/// keeps a needle that occurs almost everywhere from costing a multiple of the
/// input in indices.
#[must_use]
pub fn occurrences<'a>(haystack: &'a [u8], needle: &'a [u8]) -> Occurrences<'a> {
    occurrences_by_span(haystack, needle, FIND_ALL_SPAN)
}

/// [`occurrences`] over a span the caller names, so the boundary between two
/// spans can be tested without an input the size of a real one.
fn occurrences_by_span<'a>(haystack: &'a [u8], needle: &'a [u8], span: usize) -> Occurrences<'a> {
    Occurrences { haystack, needle, span: span.max(1), from: 0, buf: Vec::new(), taken: 0 }
}

/// Every position at which `needle` occurs, each found when the walk asks for
/// it, holding nothing between them.
///
/// The same positions [`occurrences`] yields, reached by resuming the search a
/// byte past each hit rather than by searching a span ahead. It allocates
/// nothing and splits nothing, which is what makes it the arm to price the span
/// walk against: a route can take either, in one process, in one rotation, so
/// the difference read between them is the two forms and not two runs.
#[must_use]
pub fn occurrences_unsplit<'a>(haystack: &'a [u8], needle: &'a [u8]) -> Unsplit<'a> {
    Unsplit { haystack, needle, from: 0 }
}

/// The walk [`occurrences_unsplit`] returns.
pub struct Unsplit<'a> {
    haystack: &'a [u8],
    needle: &'a [u8],
    from: usize,
}

impl Iterator for Unsplit<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        if self.needle.is_empty() || self.from >= self.haystack.len() {
            return None;
        }
        let rel = find(&self.haystack[self.from..], self.needle)?;
        let at = self.from + rel;
        self.from = at + 1;
        Some(at)
    }
}

/// The walk [`occurrences`] returns.
pub struct Occurrences<'a> {
    haystack: &'a [u8],
    needle: &'a [u8],
    /// How much of the haystack one span covers.
    span: usize,
    /// Where the next span begins.
    from: usize,
    /// The current span's occurrences, absolute and ascending.
    buf: Vec<usize>,
    /// How many of `buf` have been handed out.
    taken: usize,
}

impl Iterator for Occurrences<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        let n = self.haystack.len();
        while self.taken == self.buf.len() {
            if self.needle.is_empty() || self.needle.len() > n || self.from >= n {
                return None;
            }
            // The span is widened by the needle's length less one, so an
            // occurrence starting inside it is whole in what is searched, and
            // one starting at or past its end is dropped for the next span to
            // report. This is the split's own boundary rule at span scale.
            let end = (self.from + self.span).min(n);
            let edge = (end + self.needle.len() - 1).min(n);
            let base = self.from;
            self.buf = find_all(&self.haystack[base..edge], self.needle);
            self.buf.retain(|&r| base + r < end);
            for at in &mut self.buf {
                *at += base;
            }
            self.taken = 0;
            self.from = end;
        }
        let at = self.buf[self.taken];
        self.taken += 1;
        Some(at)
    }
}

/// [`find_all`] in one pass whatever the size, the form the crossover bench
/// times the split against.
#[must_use]
pub fn find_all_one_pass(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let mut from = 0usize;
    while let Some(rel) = find(&haystack[from..], needle) {
        let at = from + rel;
        out.push(at);
        from = at + 1;
    }
    out
}

/// [`find_all`] with the haystack split across the cores.
///
/// Each leaf searches its own slice widened by `needle.len() - 1` on the far
/// side, so an occurrence straddling a boundary is whole in the slice it starts
/// in, and is reported by that leaf alone because a hit at or past the leaf's
/// own end is dropped. The leaves' results concatenate in order because their
/// slices do.
///
/// Always parallel. [`find_all`] holds the size below which splitting costs
/// more than searching, measured rather than assumed.
#[must_use]
pub fn find_all_across(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    let n = haystack.len();
    if needle.is_empty() || needle.len() > n {
        return Vec::new();
    }
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let width = n.div_ceil(cores * 4).max(needle.len());
    let leaves = n.div_ceil(width);
    // The leaf count, because it is what the split costs rather than what it
    // saves: each leaf holds its own vector and they are joined by a copy, so a
    // reading of this route that does not know the count cannot tell a search
    // that ran too slowly from a join that ran too often.
    crate::trace::rung("find all across", "leaves", leaves);
    let mut found: Vec<Vec<usize>> = vec![Vec::new(); leaves];
    // `Streaming` is the scheduler's own name for this work - its table reads
    // "per-core bandwidth-bound: byte scan" - and it carries the SMT, cost and
    // oversubscription settings together. The batch is the number of items the
    // closure below iterates, which is the leaves, and no cost is pinned: an
    // explicit per-element cost replaces the probe that would otherwise measure
    // this workload, and what a leaf costs is exactly what is in question here.
    let plan = flynnel::JobPlan::set_profile(
        0,
        u32::try_from(leaves).unwrap_or(u32::MAX),
        flynnel::DispatchProfile::Streaming,
    );
    flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf(&plan, &mut found, 1, |base, slots| {
        for (k, slot) in slots.iter_mut().enumerate() {
            let lo = (base + k) * width;
            let hi = (lo + width).min(n);
            let edge = (hi + needle.len() - 1).min(n);
            // The one-pass form by name: a leaf is a slice of the split this
            // call already made, and searching it through the choosing form
            // would split it again.
            *slot = find_all_one_pass(&haystack[lo..edge], needle)
                .into_iter()
                .map(|r| lo + r)
                .filter(|&at| at < hi)
                .collect();
        }
    });
    found.concat()
}

/// The scalar baseline of [`find`], exposed so a measurement can
/// compare it against the dispatched SIMD path.
#[must_use]
pub fn find_scalar(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > haystack.len() {
        return None;
    }
    let first = needle[0];
    let last = haystack.len() - needle.len();
    (0..=last).find(|&i| haystack[i] == first && &haystack[i..i + needle.len()] == needle)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn find_avx2(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    use core::arch::x86_64::{
        _mm256_cmpeq_epi8, _mm256_loadu_si256, _mm256_movemask_epi8, _mm256_set1_epi8,
    };
    let n = needle.len();
    let last = haystack.len() - n;
    let first = _mm256_set1_epi8(needle[0] as i8);
    let final_byte = _mm256_set1_epi8(needle[n - 1] as i8);
    let mut i = 0;
    while i + 32 <= haystack.len() {
        // SAFETY: the load reads 32 bytes from `i`, and `i + 32 <=
        // haystack.len()` is the loop guard.
        let chunk = unsafe { _mm256_loadu_si256(haystack.as_ptr().add(i).cast()) };
        let mut mask = _mm256_movemask_epi8(_mm256_cmpeq_epi8(chunk, first)) as u32;
        // A second load at the needle's last byte turns candidates away
        // wholesale, and there is nothing to turn away when the first byte
        // matched nowhere in this window.
        if mask != 0 && i + n - 1 + 32 <= haystack.len() {
            // SAFETY: the load reads 32 bytes from `i + n - 1`, which the
            // guard above bounds.
            let tail = unsafe { _mm256_loadu_si256(haystack.as_ptr().add(i + n - 1).cast()) };
            mask &= _mm256_movemask_epi8(_mm256_cmpeq_epi8(tail, final_byte)) as u32;
        }
        while mask != 0 {
            let pos = i + mask.trailing_zeros() as usize;
            if pos <= last && &haystack[pos..pos + n] == needle {
                return Some(pos);
            }
            mask &= mask - 1;
        }
        i += 32;
    }
    while i <= last {
        if haystack[i] == needle[0] && &haystack[i..i + n] == needle {
            return Some(i);
        }
        i += 1;
    }
    None
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn find_sse2(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    use core::arch::x86_64::{
        _mm_cmpeq_epi8, _mm_loadu_si128, _mm_movemask_epi8, _mm_set1_epi8,
    };
    let n = needle.len();
    let last = haystack.len() - n;
    let first = _mm_set1_epi8(needle[0] as i8);
    let final_byte = _mm_set1_epi8(needle[n - 1] as i8);
    let mut i = 0;
    while i + 16 <= haystack.len() {
        // SAFETY: the load reads 16 bytes from `i`, and `i + 16 <=
        // haystack.len()` is the loop guard.
        let chunk = unsafe { _mm_loadu_si128(haystack.as_ptr().add(i).cast()) };
        let mut mask = _mm_movemask_epi8(_mm_cmpeq_epi8(chunk, first)) as u32;
        // A second load at the needle's last byte turns candidates away
        // wholesale, and there is nothing to turn away when the first byte
        // matched nowhere in this window.
        if mask != 0 && i + n - 1 + 16 <= haystack.len() {
            // SAFETY: the load reads 16 bytes from `i + n - 1`, which the
            // guard above bounds.
            let tail = unsafe { _mm_loadu_si128(haystack.as_ptr().add(i + n - 1).cast()) };
            mask &= _mm_movemask_epi8(_mm_cmpeq_epi8(tail, final_byte)) as u32;
        }
        while mask != 0 {
            let pos = i + mask.trailing_zeros() as usize;
            if pos <= last && &haystack[pos..pos + n] == needle {
                return Some(pos);
            }
            mask &= mask - 1;
        }
        i += 16;
    }
    while i <= last {
        if haystack[i] == needle[0] && &haystack[i..i + n] == needle {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// AVX-512 search: probe the first needle byte 64 positions at a time with a
/// single masked compare, turn a candidate away on the needle's last byte,
/// and verify the full needle at what is left. Twice the window of the AVX2
/// path.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn find_avx512(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    use core::arch::x86_64::{_mm512_cmpeq_epi8_mask, _mm512_loadu_si512, _mm512_set1_epi8};
    let n = needle.len();
    let last = haystack.len() - n;
    let first = _mm512_set1_epi8(needle[0] as i8);
    let final_byte = _mm512_set1_epi8(needle[n - 1] as i8);
    let mut i = 0;
    while i + 64 <= haystack.len() {
        // SAFETY: the load reads 64 bytes from `i`, and `i + 64 <=
        // haystack.len()` is the loop guard.
        let chunk = unsafe { _mm512_loadu_si512(haystack.as_ptr().add(i).cast()) };
        let mut mask: u64 = _mm512_cmpeq_epi8_mask(chunk, first);
        // A second load at the needle's last byte turns candidates away
        // wholesale, and there is nothing to turn away when the first byte
        // matched nowhere in this window - which is the whole of the window
        // on a needle the input does not hold. So the window that found
        // nothing costs one load, and only a window with something to filter
        // runs the filter.
        if mask != 0 && i + n - 1 + 64 <= haystack.len() {
            // SAFETY: the load reads 64 bytes from `i + n - 1`, which the
            // guard above bounds.
            let tail = unsafe { _mm512_loadu_si512(haystack.as_ptr().add(i + n - 1).cast()) };
            mask &= _mm512_cmpeq_epi8_mask(tail, final_byte);
        }
        while mask != 0 {
            let pos = i + mask.trailing_zeros() as usize;
            if pos <= last && &haystack[pos..pos + n] == needle {
                return Some(pos);
            }
            mask &= mask - 1;
        }
        i += 64;
    }
    while i <= last {
        if haystack[i] == needle[0] && &haystack[i..i + n] == needle {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// A scalar emulation of the exact `find_avx512` algorithm: the same 64-byte
/// first-byte probe, the same last-byte rejection, the same masked candidate
/// walk, the same verify, but the 64-bit match mask is built with scalar
/// comparisons so it runs on any CPU. This is the verification path for hosts
/// without AVX-512 (it is slow by design): a test asserts it is
/// byte-identical to the scalar baseline, which proves the 64-wide algorithm
/// is exact even where the real AVX-512 instructions cannot be executed.
#[must_use]
pub fn find_avx512_emulated(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > haystack.len() {
        return None;
    }
    let n = needle.len();
    let last = haystack.len() - n;
    let first = needle[0];
    let final_byte = needle[n - 1];
    let mut i = 0;
    while i + 64 <= haystack.len() {
        // The mask an AVX-512 byte compare would produce for this window.
        let mut mask: u64 = 0;
        for j in 0..64 {
            if haystack[i + j] == first {
                mask |= 1u64 << j;
            }
        }
        // The second compare, taken only where the first found something,
        // exactly as the vector path takes its second load.
        if mask != 0 && i + n - 1 + 64 <= haystack.len() {
            let mut tail: u64 = 0;
            for j in 0..64 {
                if haystack[i + n - 1 + j] == final_byte {
                    tail |= 1u64 << j;
                }
            }
            mask &= tail;
        }
        while mask != 0 {
            let pos = i + mask.trailing_zeros() as usize;
            if pos <= last && &haystack[pos..pos + n] == needle {
                return Some(pos);
            }
            mask &= mask - 1;
        }
        i += 64;
    }
    while i <= last {
        if haystack[i] == first && &haystack[i..i + n] == needle {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// The position of the `n`th occurrence of `byte` in `haystack`, counting
/// from one at the start, or `None` where it holds fewer than `n` or `n` is
/// zero. The first `n` lines of an input end at its `n`th newline.
#[must_use]
pub fn nth_byte(haystack: &[u8], byte: u8, n: usize) -> Option<usize> {
    if n == 0 {
        return None;
    }
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY on each arm: `crate::isa::tier` reports the rungs this CPU
        // can run, resolved once for the process, and a rung implies every
        // rung below it.
        match crate::isa::tier() {
            crate::isa::Tier::Avx512 => return unsafe { nth_byte_avx512(haystack, byte, n) },
            crate::isa::Tier::Avx2 => return unsafe { nth_byte_avx2(haystack, byte, n) },
            crate::isa::Tier::Sse2 => return unsafe { nth_byte_sse2(haystack, byte, n) },
            crate::isa::Tier::Scalar => {}
        }
    }
    nth_byte_scalar(haystack, byte, n)
}

/// The scalar baseline of [`nth_byte`].
#[must_use]
pub fn nth_byte_scalar(haystack: &[u8], byte: u8, n: usize) -> Option<usize> {
    let skip = n.checked_sub(1)?;
    haystack.iter().enumerate().filter(|&(_, &b)| b == byte).nth(skip).map(|(i, _)| i)
}

/// The position of the `n`th occurrence of `byte` in `haystack`, counting
/// from one at the end, or `None` where it holds fewer than `n` or `n` is
/// zero. The last lines of an input are found from its end this way, reading
/// only the bytes they span.
#[must_use]
pub fn nth_byte_back(haystack: &[u8], byte: u8, n: usize) -> Option<usize> {
    if n == 0 {
        return None;
    }
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY on each arm: as in `nth_byte`.
        match crate::isa::tier() {
            crate::isa::Tier::Avx512 => return unsafe { nth_byte_back_avx512(haystack, byte, n) },
            crate::isa::Tier::Avx2 => return unsafe { nth_byte_back_avx2(haystack, byte, n) },
            crate::isa::Tier::Sse2 => return unsafe { nth_byte_back_sse2(haystack, byte, n) },
            crate::isa::Tier::Scalar => {}
        }
    }
    nth_byte_back_scalar(haystack, byte, n)
}

/// The scalar baseline of [`nth_byte_back`].
#[must_use]
pub fn nth_byte_back_scalar(haystack: &[u8], byte: u8, n: usize) -> Option<usize> {
    let skip = n.checked_sub(1)?;
    haystack.iter().enumerate().rev().filter(|&(_, &b)| b == byte).nth(skip).map(|(i, _)| i)
}

/// How many times `byte` occurs in `haystack`, in one pass. The lines ahead
/// of a window are counted this way where its line numbers are printed.
#[must_use]
pub fn count_byte(haystack: &[u8], byte: u8) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY on each arm: as in `nth_byte`.
        match crate::isa::tier() {
            crate::isa::Tier::Avx512 => return unsafe { count_byte_avx512(haystack, byte) },
            crate::isa::Tier::Avx2 => return unsafe { count_byte_avx2(haystack, byte) },
            crate::isa::Tier::Sse2 => return unsafe { count_byte_sse2(haystack, byte) },
            crate::isa::Tier::Scalar => {}
        }
    }
    count_byte_scalar(haystack, byte)
}

/// The scalar baseline of [`count_byte`].
#[must_use]
pub fn count_byte_scalar(haystack: &[u8], byte: u8) -> usize {
    haystack.iter().filter(|&&b| b == byte).count()
}

/// [`count_byte`] with the haystack split across the cores, each leaf
/// counting its own slice. Always split; [`count_byte_split`] splits only
/// from the size at which `benches/count_byte_crossover` measured it winning.
#[must_use]
pub fn count_byte_across(haystack: &[u8], byte: u8) -> usize {
    let n = haystack.len();
    if n == 0 {
        return 0;
    }
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let width = n.div_ceil(cores * 4).max(1);
    let leaves = n.div_ceil(width);
    crate::trace::rung("count byte across", "leaves", leaves);
    let mut counts = vec![0usize; leaves];
    // The byte scan's own profile, as `find_all_across` takes it, and no cost
    // pinned, since what a leaf costs is what the crossover bench measures.
    let plan = flynnel::JobPlan::set_profile(
        0,
        u32::try_from(leaves).expect("the leaves number at most four a core"),
        flynnel::DispatchProfile::Streaming,
    );
    flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf(&plan, &mut counts, 1, |base, slots| {
        for (k, slot) in slots.iter_mut().enumerate() {
            let lo = (base + k) * width;
            let hi = (lo + width).min(n);
            *slot = count_byte(&haystack[lo..hi], byte);
        }
    });
    counts.iter().sum()
}

/// The size from which [`count_byte_split`] splits a count across the cores.
/// `benches/count_byte_crossover` read the split at 0.39-0.49x the one pass at
/// 1 MiB and 1.17-1.28x at 4 MiB, over its corpus and six real files and for
/// both bytes it counts, on a Ryzen 9 7900X with 24 threads.
pub const SPLIT_FROM: usize = 4 * 1024 * 1024;

/// How many times `byte` occurs in `haystack`: [`count_byte`] below
/// [`SPLIT_FROM`] bytes and [`count_byte_across`] from it. For a count on the
/// calling thread over text held in memory whole. A count that already runs
/// on a core of its own takes [`count_byte`], and so does a reader counting
/// each block it reads: there the read is the bottleneck, and a one-shot trex
/// run measured the split 7 to 11% slower on ranges read from 12 to 131 MB
/// files, the pool's start included.
#[must_use]
pub fn count_byte_split(haystack: &[u8], byte: u8) -> usize {
    if haystack.len() < SPLIT_FROM { count_byte(haystack, byte) } else { count_byte_across(haystack, byte) }
}

/// The position in a block of up to 64 bytes of the `k`th set bit of `mask`,
/// counting from one at the lowest; `mask` holds at least `k` set bits.
#[inline]
fn nth_set_bit(mut mask: u64, k: usize) -> usize {
    for _ in 1..k {
        mask &= mask - 1;
    }
    mask.trailing_zeros() as usize
}

/// The position in a block of the `k`th set bit of `mask`, counting from one
/// at the highest; `mask` holds at least `k` set bits.
#[inline]
fn nth_set_bit_from_top(mut mask: u64, k: usize) -> usize {
    for _ in 1..k {
        mask ^= 1u64 << (63 - mask.leading_zeros());
    }
    (63 - mask.leading_zeros()) as usize
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn nth_byte_avx2(haystack: &[u8], byte: u8, n: usize) -> Option<usize> {
    use core::arch::x86_64::{_mm256_cmpeq_epi8, _mm256_loadu_si256, _mm256_movemask_epi8, _mm256_set1_epi8};
    let wanted = _mm256_set1_epi8(byte as i8);
    let mut left = n;
    let mut i = 0;
    while i + 32 <= haystack.len() {
        // SAFETY: the load reads 32 bytes from `i`, and `i + 32 <=
        // haystack.len()` is the loop guard.
        let chunk = unsafe { _mm256_loadu_si256(haystack.as_ptr().add(i).cast()) };
        let mask = u64::from(_mm256_movemask_epi8(_mm256_cmpeq_epi8(chunk, wanted)) as u32);
        let here = mask.count_ones() as usize;
        if here >= left {
            return Some(i + nth_set_bit(mask, left));
        }
        left -= here;
        i += 32;
    }
    nth_byte_scalar(&haystack[i..], byte, left).map(|at| i + at)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn nth_byte_sse2(haystack: &[u8], byte: u8, n: usize) -> Option<usize> {
    use core::arch::x86_64::{_mm_cmpeq_epi8, _mm_loadu_si128, _mm_movemask_epi8, _mm_set1_epi8};
    let wanted = _mm_set1_epi8(byte as i8);
    let mut left = n;
    let mut i = 0;
    while i + 16 <= haystack.len() {
        // SAFETY: the load reads 16 bytes from `i`, and `i + 16 <=
        // haystack.len()` is the loop guard.
        let chunk = unsafe { _mm_loadu_si128(haystack.as_ptr().add(i).cast()) };
        let mask = u64::from(_mm_movemask_epi8(_mm_cmpeq_epi8(chunk, wanted)) as u32);
        let here = mask.count_ones() as usize;
        if here >= left {
            return Some(i + nth_set_bit(mask, left));
        }
        left -= here;
        i += 16;
    }
    nth_byte_scalar(&haystack[i..], byte, left).map(|at| i + at)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn nth_byte_avx512(haystack: &[u8], byte: u8, n: usize) -> Option<usize> {
    use core::arch::x86_64::{_mm512_cmpeq_epi8_mask, _mm512_loadu_si512, _mm512_set1_epi8};
    let wanted = _mm512_set1_epi8(byte as i8);
    let mut left = n;
    let mut i = 0;
    while i + 64 <= haystack.len() {
        // SAFETY: the load reads 64 bytes from `i`, and `i + 64 <=
        // haystack.len()` is the loop guard.
        let chunk = unsafe { _mm512_loadu_si512(haystack.as_ptr().add(i).cast()) };
        let mask: u64 = _mm512_cmpeq_epi8_mask(chunk, wanted);
        let here = mask.count_ones() as usize;
        if here >= left {
            return Some(i + nth_set_bit(mask, left));
        }
        left -= here;
        i += 64;
    }
    nth_byte_scalar(&haystack[i..], byte, left).map(|at| i + at)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn nth_byte_back_avx2(haystack: &[u8], byte: u8, n: usize) -> Option<usize> {
    use core::arch::x86_64::{_mm256_cmpeq_epi8, _mm256_loadu_si256, _mm256_movemask_epi8, _mm256_set1_epi8};
    let wanted = _mm256_set1_epi8(byte as i8);
    let mut left = n;
    let mut end = haystack.len();
    while end >= 32 {
        let start = end - 32;
        // SAFETY: the load reads 32 bytes from `start`, which is `end - 32`,
        // and `end` never exceeds `haystack.len()`.
        let chunk = unsafe { _mm256_loadu_si256(haystack.as_ptr().add(start).cast()) };
        let mask = u64::from(_mm256_movemask_epi8(_mm256_cmpeq_epi8(chunk, wanted)) as u32);
        let here = mask.count_ones() as usize;
        if here >= left {
            return Some(start + nth_set_bit_from_top(mask, left));
        }
        left -= here;
        end = start;
    }
    nth_byte_back_scalar(&haystack[..end], byte, left)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn nth_byte_back_sse2(haystack: &[u8], byte: u8, n: usize) -> Option<usize> {
    use core::arch::x86_64::{_mm_cmpeq_epi8, _mm_loadu_si128, _mm_movemask_epi8, _mm_set1_epi8};
    let wanted = _mm_set1_epi8(byte as i8);
    let mut left = n;
    let mut end = haystack.len();
    while end >= 16 {
        let start = end - 16;
        // SAFETY: the load reads 16 bytes from `start`, which is `end - 16`,
        // and `end` never exceeds `haystack.len()`.
        let chunk = unsafe { _mm_loadu_si128(haystack.as_ptr().add(start).cast()) };
        let mask = u64::from(_mm_movemask_epi8(_mm_cmpeq_epi8(chunk, wanted)) as u32);
        let here = mask.count_ones() as usize;
        if here >= left {
            return Some(start + nth_set_bit_from_top(mask, left));
        }
        left -= here;
        end = start;
    }
    nth_byte_back_scalar(&haystack[..end], byte, left)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn nth_byte_back_avx512(haystack: &[u8], byte: u8, n: usize) -> Option<usize> {
    use core::arch::x86_64::{_mm512_cmpeq_epi8_mask, _mm512_loadu_si512, _mm512_set1_epi8};
    let wanted = _mm512_set1_epi8(byte as i8);
    let mut left = n;
    let mut end = haystack.len();
    while end >= 64 {
        let start = end - 64;
        // SAFETY: the load reads 64 bytes from `start`, which is `end - 64`,
        // and `end` never exceeds `haystack.len()`.
        let chunk = unsafe { _mm512_loadu_si512(haystack.as_ptr().add(start).cast()) };
        let mask: u64 = _mm512_cmpeq_epi8_mask(chunk, wanted);
        let here = mask.count_ones() as usize;
        if here >= left {
            return Some(start + nth_set_bit_from_top(mask, left));
        }
        left -= here;
        end = start;
    }
    nth_byte_back_scalar(&haystack[..end], byte, left)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn count_byte_avx2(haystack: &[u8], byte: u8) -> usize {
    use core::arch::x86_64::{_mm256_cmpeq_epi8, _mm256_loadu_si256, _mm256_movemask_epi8, _mm256_set1_epi8};
    let wanted = _mm256_set1_epi8(byte as i8);
    let mut count = 0usize;
    let mut i = 0;
    while i + 32 <= haystack.len() {
        // SAFETY: the load reads 32 bytes from `i`, and `i + 32 <=
        // haystack.len()` is the loop guard.
        let chunk = unsafe { _mm256_loadu_si256(haystack.as_ptr().add(i).cast()) };
        count += (_mm256_movemask_epi8(_mm256_cmpeq_epi8(chunk, wanted)) as u32).count_ones() as usize;
        i += 32;
    }
    count + count_byte_scalar(&haystack[i..], byte)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn count_byte_sse2(haystack: &[u8], byte: u8) -> usize {
    use core::arch::x86_64::{_mm_cmpeq_epi8, _mm_loadu_si128, _mm_movemask_epi8, _mm_set1_epi8};
    let wanted = _mm_set1_epi8(byte as i8);
    let mut count = 0usize;
    let mut i = 0;
    while i + 16 <= haystack.len() {
        // SAFETY: the load reads 16 bytes from `i`, and `i + 16 <=
        // haystack.len()` is the loop guard.
        let chunk = unsafe { _mm_loadu_si128(haystack.as_ptr().add(i).cast()) };
        count += (_mm_movemask_epi8(_mm_cmpeq_epi8(chunk, wanted)) as u32).count_ones() as usize;
        i += 16;
    }
    count + count_byte_scalar(&haystack[i..], byte)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn count_byte_avx512(haystack: &[u8], byte: u8) -> usize {
    use core::arch::x86_64::{_mm512_cmpeq_epi8_mask, _mm512_loadu_si512, _mm512_set1_epi8};
    let wanted = _mm512_set1_epi8(byte as i8);
    let mut count = 0usize;
    let mut i = 0;
    while i + 64 <= haystack.len() {
        // SAFETY: the load reads 64 bytes from `i`, and `i + 64 <=
        // haystack.len()` is the loop guard.
        let chunk = unsafe { _mm512_loadu_si512(haystack.as_ptr().add(i).cast()) };
        count += _mm512_cmpeq_epi8_mask(chunk, wanted).count_ones() as usize;
        i += 64;
    }
    count + count_byte_scalar(&haystack[i..], byte)
}

/// Length of the leading run of "word" bytes -- ASCII alphanumerics and
/// `_` -- at the start of `b`. The lexer scans an identifier this way; the
/// SIMD path classifies 32 bytes per step and stops at the first non-word
/// byte, where the scalar loop tested one byte per comparison. A byte
/// `>= 128` is a negative `i8`, below every ASCII threshold, so it is
/// non-word -- matching the scalar `is_ascii_alphanumeric` exactly. The
/// lexer's word loop decodes the UTF-8 char at that stop and continues the
/// run while it is a letter, so this stays the pure-ASCII fast path.
#[must_use]
#[inline]
pub fn word_run(b: &[u8]) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY on each arm: the tier is resolved once per process from the
        // CPU's own feature report, and a rung implies every rung below it,
        // so reaching an arm is the guarantee that the features its body was
        // compiled for are present.
        if crate::isa::tier() >= crate::isa::Tier::Avx512 {
            return unsafe { word_run_avx512(b) };
        }
        if crate::isa::tier() >= crate::isa::Tier::Avx2 {
            return unsafe { word_run_avx2(b) };
        }
    }
    word_run_scalar(b)
}

/// Scalar reference for [`word_run`].
#[must_use]
pub fn word_run_scalar(b: &[u8]) -> usize {
    b.iter().take_while(|&&c| c == b'_' || c.is_ascii_alphanumeric()).count()
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn word_run_avx2(b: &[u8]) -> usize {
    use core::arch::x86_64::{
        _mm256_and_si256, _mm256_cmpeq_epi8, _mm256_cmpgt_epi8, _mm256_loadu_si256,
        _mm256_movemask_epi8, _mm256_or_si256, _mm256_set1_epi8,
    };
    let n = b.len();
    let mut i = 0;
    while i + 32 <= n {
        // SAFETY: the 32-byte load is bounded by `i + 32 <= n`.
        let v = unsafe { _mm256_loadu_si256(b.as_ptr().add(i).cast()) };
        // Each class is a half-open ASCII range tested with two signed
        // compares; `'a'..='z'` is `b > 96 && 123 > b`, and so on.
        let lower = _mm256_and_si256(
            _mm256_cmpgt_epi8(v, _mm256_set1_epi8(96)),
            _mm256_cmpgt_epi8(_mm256_set1_epi8(123), v),
        );
        let upper = _mm256_and_si256(
            _mm256_cmpgt_epi8(v, _mm256_set1_epi8(64)),
            _mm256_cmpgt_epi8(_mm256_set1_epi8(91), v),
        );
        let digit = _mm256_and_si256(
            _mm256_cmpgt_epi8(v, _mm256_set1_epi8(47)),
            _mm256_cmpgt_epi8(_mm256_set1_epi8(58), v),
        );
        let under = _mm256_cmpeq_epi8(v, _mm256_set1_epi8(95));
        let word = _mm256_or_si256(_mm256_or_si256(lower, upper), _mm256_or_si256(digit, under));
        let mask = _mm256_movemask_epi8(word) as u32;
        if mask != 0xFFFF_FFFF {
            // The lowest zero bit is the first non-word byte in this window.
            return i + (!mask).trailing_zeros() as usize;
        }
        i += 32;
    }
    i + word_run_scalar(&b[i..])
}

/// [`word_run_avx2`] over 64 bytes a step, the compare yielding the mask
/// itself rather than a vector to be moved into one.
///
/// This is called with the rest of the input and stops at the first byte
/// outside the class, so a token shorter than a window is answered by one
/// window whatever the window's width. `examples/simd_search` reads it against
/// the 32-byte step on tokens either side of both widths.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn word_run_avx512(b: &[u8]) -> usize {
    use core::arch::x86_64::{
        _mm512_cmpeq_epi8_mask, _mm512_cmpgt_epi8_mask, _mm512_loadu_si512, _mm512_set1_epi8,
    };
    let n = b.len();
    let mut i = 0;
    while i + 64 <= n {
        // SAFETY: the 64-byte load is bounded by `i + 64 <= n`.
        let v = unsafe { _mm512_loadu_si512(b.as_ptr().add(i).cast()) };
        // Each class is a half-open ASCII range tested with two signed
        // compares; `'a'..='z'` is `b > 96 && 123 > b`, and so on.
        let lower = _mm512_cmpgt_epi8_mask(v, _mm512_set1_epi8(96))
            & _mm512_cmpgt_epi8_mask(_mm512_set1_epi8(123), v);
        let upper = _mm512_cmpgt_epi8_mask(v, _mm512_set1_epi8(64))
            & _mm512_cmpgt_epi8_mask(_mm512_set1_epi8(91), v);
        let digit = _mm512_cmpgt_epi8_mask(v, _mm512_set1_epi8(47))
            & _mm512_cmpgt_epi8_mask(_mm512_set1_epi8(58), v);
        let under = _mm512_cmpeq_epi8_mask(v, _mm512_set1_epi8(95));
        let word: u64 = lower | upper | digit | under;
        if word != u64::MAX {
            // The lowest zero bit is the first non-word byte in this window.
            return i + (!word).trailing_zeros() as usize;
        }
        i += 64;
    }
    i + word_run_scalar(&b[i..])
}

/// Length of the leading run of ASCII whitespace bytes (the five
/// `is_ascii_whitespace` bytes: tab, newline, form feed, carriage return,
/// space). The lexer skips an inter-token whitespace run this way.
#[must_use]
#[inline]
pub fn space_run(b: &[u8]) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY on each arm: the tier is resolved once per process from the
        // CPU's own feature report, and a rung implies every rung below it,
        // so reaching an arm is the guarantee that the features its body was
        // compiled for are present.
        if crate::isa::tier() >= crate::isa::Tier::Avx512 {
            return unsafe { space_run_avx512(b) };
        }
        if crate::isa::tier() >= crate::isa::Tier::Avx2 {
            return unsafe { space_run_avx2(b) };
        }
    }
    space_run_scalar(b)
}

/// Scalar reference for [`space_run`].
#[must_use]
pub fn space_run_scalar(b: &[u8]) -> usize {
    b.iter().take_while(|&&c| c.is_ascii_whitespace()).count()
}

/// Length of the leading run of bytes that are not ASCII whitespace: the
/// span between two whitespace bytes, which the entropy pre-pass reads as
/// one unit, since a blob run never holds whitespace.
#[must_use]
#[inline]
pub fn nonspace_run(b: &[u8]) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY on each arm: the tier is resolved once per process from the
        // CPU's own feature report, and a rung implies every rung below it,
        // so reaching an arm is the guarantee that the features its body was
        // compiled for are present.
        if crate::isa::tier() >= crate::isa::Tier::Avx512 {
            return unsafe { nonspace_run_avx512(b) };
        }
        if crate::isa::tier() >= crate::isa::Tier::Avx2 {
            return unsafe { nonspace_run_avx2(b) };
        }
    }
    nonspace_run_scalar(b)
}

/// Scalar reference for [`nonspace_run`].
#[must_use]
pub fn nonspace_run_scalar(b: &[u8]) -> usize {
    b.iter().take_while(|&&c| !c.is_ascii_whitespace()).count()
}

/// The maximal runs of bytes that are not ASCII whitespace in `b`, at least
/// `min_len` long and never empty, as ascending `b`-relative ranges. The
/// entropy pre-pass reads these and nothing else.
#[must_use]
pub fn nonspace_spans_at_least(b: &[u8], min_len: usize) -> Vec<(usize, usize)> {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY on each arm: the tier is resolved once per process from the
        // CPU's own feature report, and a rung implies every rung below it,
        // so reaching an arm is the guarantee that the features its body was
        // compiled for are present.
        if crate::isa::tier() >= crate::isa::Tier::Avx512 {
            return unsafe { nonspace_spans_at_least_avx512(b, min_len) };
        }
        if crate::isa::tier() >= crate::isa::Tier::Avx2 {
            return unsafe { nonspace_spans_at_least_avx2(b, min_len) };
        }
    }
    nonspace_spans_at_least_scalar(b, min_len)
}

/// Scalar reference for [`nonspace_spans_at_least`].
#[must_use]
pub fn nonspace_spans_at_least_scalar(b: &[u8], min_len: usize) -> Vec<(usize, usize)> {
    let min_len = min_len.max(1);
    let mut spans = Vec::new();
    let mut start = 0;
    for (j, &c) in b.iter().enumerate() {
        if c.is_ascii_whitespace() {
            if j - start >= min_len {
                spans.push((start, j));
            }
            start = j + 1;
        }
    }
    if b.len() - start >= min_len {
        spans.push((start, b.len()));
    }
    spans
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn nonspace_spans_at_least_avx2(b: &[u8], min_len: usize) -> Vec<(usize, usize)> {
    use core::arch::x86_64::_mm256_loadu_si256;
    let min_len = min_len.max(1);
    let n = b.len();
    let mut spans = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i + 32 <= n {
        // SAFETY: the 32-byte load is bounded by `i + 32 <= n`, and the
        // lane test runs under the same feature as this function.
        let mask = unsafe { whitespace_lanes(_mm256_loadu_si256(b.as_ptr().add(i).cast())) };
        if mask != 0 {
            // The run reaching the chunk's first whitespace byte; then the
            // runs between whitespace bytes inside the chunk, which reach
            // `min_len` only when it is under the chunk width; then the run
            // opening after the chunk's last whitespace byte.
            let first = mask.trailing_zeros() as usize;
            if i + first - start >= min_len {
                spans.push((start, i + first));
            }
            // A run between two whitespace bytes of this chunk needs `min_len`
            // zero lanes between them, so a chunk holding fewer zeros than
            // that in total cannot hold one and the walk would push nothing.
            // The walk costs an iteration per whitespace byte, which is what
            // text with whitespace every few bytes gives it.
            if min_len < 32 && 32 - mask.count_ones() as usize >= min_len {
                let mut rest = mask & (mask - 1);
                let mut prev = first;
                while rest != 0 {
                    let q = rest.trailing_zeros() as usize;
                    if q - prev > min_len {
                        spans.push((i + prev + 1, i + q));
                    }
                    prev = q;
                    rest &= rest - 1;
                }
            }
            start = i + 32 - mask.leading_zeros() as usize;
        }
        i += 32;
    }
    for (j, &c) in b.iter().enumerate().skip(i) {
        if c.is_ascii_whitespace() {
            if j - start >= min_len {
                spans.push((start, j));
            }
            start = j + 1;
        }
    }
    if n - start >= min_len {
        spans.push((start, n));
    }
    spans
}

/// [`nonspace_spans_at_least_avx2`] over 64 bytes a step. The whole input is
/// classified here, once per scan, where a run function reads one window per
/// token: the wider step is the gain.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn nonspace_spans_at_least_avx512(b: &[u8], min_len: usize) -> Vec<(usize, usize)> {
    use core::arch::x86_64::_mm512_loadu_si512;
    let min_len = min_len.max(1);
    let n = b.len();
    let mut spans = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i + 64 <= n {
        // SAFETY: the 64-byte load is bounded by `i + 64 <= n`, and the lane
        // test runs under the same features as this function.
        let mask =
            unsafe { whitespace_lanes_512(_mm512_loadu_si512(b.as_ptr().add(i).cast())) };
        if mask != 0 {
            // The run reaching the chunk's first whitespace byte; then the
            // runs between whitespace bytes inside the chunk, which reach
            // `min_len` only when it is under the chunk width; then the run
            // opening after the chunk's last whitespace byte.
            let first = mask.trailing_zeros() as usize;
            if i + first - start >= min_len {
                spans.push((start, i + first));
            }
            // A run between two whitespace bytes of this chunk needs `min_len`
            // zero lanes between them, so a chunk holding fewer zeros than that
            // in total cannot hold one anywhere and the walk below would push
            // nothing. The walk costs an iteration per whitespace byte, and
            // text with whitespace every few bytes - which is what code is -
            // reaches that condition on most chunks.
            //
            // What the guard is worth, over 3,688,563 bytes of this crate's own
            // src/*.rs, arms alternated four rounds with `leaves` as the
            // control: this scan 1084.06 -> 539.74 us at 4 MB, the blob
            // pre-pass around it -41.5%, and the fused significant lex 3.76 ->
            // 3.12 ms, a ratio of 0.830. The saving is a property of the bytes,
            // not of the guard: it saves time where whitespace is dense and
            // saves nothing on a corpus of long unbroken runs.
            if min_len < 64 && 64 - mask.count_ones() as usize >= min_len {
                let mut rest = mask & (mask - 1);
                let mut prev = first;
                while rest != 0 {
                    let q = rest.trailing_zeros() as usize;
                    if q - prev > min_len {
                        spans.push((i + prev + 1, i + q));
                    }
                    prev = q;
                    rest &= rest - 1;
                }
            }
            start = i + 64 - mask.leading_zeros() as usize;
        }
        i += 64;
    }
    for (j, &c) in b.iter().enumerate().skip(i) {
        if c.is_ascii_whitespace() {
            if j - start >= min_len {
                spans.push((start, j));
            }
            start = j + 1;
        }
    }
    if n - start >= min_len {
        spans.push((start, n));
    }
    spans
}

/// [`whitespace_lanes`] over 64 lanes, where the compare yields the mask
/// itself rather than a vector to be moved into one.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
#[inline]
unsafe fn whitespace_lanes_512(v: core::arch::x86_64::__m512i) -> u64 {
    use core::arch::x86_64::{_mm512_cmpeq_epi8_mask, _mm512_set1_epi8};
    _mm512_cmpeq_epi8_mask(v, _mm512_set1_epi8(9))
        | _mm512_cmpeq_epi8_mask(v, _mm512_set1_epi8(10))
        | _mm512_cmpeq_epi8_mask(v, _mm512_set1_epi8(12))
        | _mm512_cmpeq_epi8_mask(v, _mm512_set1_epi8(13))
        | _mm512_cmpeq_epi8_mask(v, _mm512_set1_epi8(32))
}

/// The lanes of `v` holding one of the five whitespace bytes, a bit per
/// lane. The five (9, 10, 12, 13, 32) are not a contiguous range - 11,
/// vertical tab, is not `is_ascii_whitespace` - so each is tested by
/// equality rather than the set by a range.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
unsafe fn whitespace_lanes(v: core::arch::x86_64::__m256i) -> u32 {
    use core::arch::x86_64::{_mm256_cmpeq_epi8, _mm256_movemask_epi8, _mm256_or_si256, _mm256_set1_epi8};
    let ws = _mm256_or_si256(
        _mm256_or_si256(
            _mm256_or_si256(
                _mm256_cmpeq_epi8(v, _mm256_set1_epi8(9)),
                _mm256_cmpeq_epi8(v, _mm256_set1_epi8(10)),
            ),
            _mm256_or_si256(
                _mm256_cmpeq_epi8(v, _mm256_set1_epi8(12)),
                _mm256_cmpeq_epi8(v, _mm256_set1_epi8(13)),
            ),
        ),
        _mm256_cmpeq_epi8(v, _mm256_set1_epi8(32)),
    );
    _mm256_movemask_epi8(ws) as u32
}

/// The positions in `b` of the bytes `X` and `Y`, ascending, each asked for
/// from any position. The bytes are classified sixty-four a step and the
/// positions read off the block's mask, so a hit costs a few instructions where
/// a search restarted at every one cost a call each. The block last classified
/// is kept, so the asks a scan makes inside one block classify it once.
pub struct PairPositions<'a, const X: u8, const Y: u8> {
    b: &'a [u8],
    tier: crate::isa::Tier,
    /// The first byte of the block `mask` covers.
    base: usize,
    /// That block's bytes equal to `X` or `Y`, a bit a byte from `base`.
    mask: u64,
}

/// The quote bytes, `"` and `'`: where a scan over strings finds each string
/// or char literal that may open, and nothing else.
pub type QuotePositions<'a> = PairPositions<'a, b'"', b'\''>;

/// The double quotes and the newlines: where a double-quoted string closes, or
/// where its line ends before it does.
pub type CloseOrNewlinePositions<'a> = PairPositions<'a, b'"', b'\n'>;

impl<'a, const X: u8, const Y: u8> PairPositions<'a, X, Y> {
    #[must_use]
    pub fn new(b: &'a [u8]) -> Self {
        let tier = crate::isa::tier();
        let mask = pair_lanes_at::<X, Y>(b, 0, tier);
        PairPositions { b, tier, base: 0, mask }
    }

    /// The first `X` or `Y` at or past `from`, or `None` where none is left.
    ///
    /// `from` may be anywhere. A scan passes over a string or a char literal
    /// by asking from its end, which is the common ask and stays in the block
    /// it read or moves on from it; one that asks behind the block it read -
    /// a string's close searched for, then the quotes inside it visited after
    /// all - costs the block holding `from` classified again.
    pub fn next_at_or_after(&mut self, from: usize) -> Option<usize> {
        let n = self.b.len();
        if from >= n {
            return None;
        }
        if from < self.base || from >= self.base + 64 {
            self.base = from & !63;
            self.mask = pair_lanes_at::<X, Y>(self.b, self.base, self.tier);
        }
        let mut mask = self.mask & (!0u64 << (from - self.base));
        while mask == 0 {
            self.base += 64;
            if self.base >= n {
                return None;
            }
            self.mask = pair_lanes_at::<X, Y>(self.b, self.base, self.tier);
            mask = self.mask;
        }
        Some(self.base + mask.trailing_zeros() as usize)
    }
}

/// The bytes equal to `X` or `Y` among `b[base..base + 64]`, a bit a byte,
/// over as many as `b` holds from `base`; the last partial block is read a byte
/// at a time, as is every block on a target with no lane test.
#[cfg_attr(not(target_arch = "x86_64"), allow(unused_variables))]
fn pair_lanes_at<const X: u8, const Y: u8>(b: &[u8], base: usize, tier: crate::isa::Tier) -> u64 {
    let end = (base + 64).min(b.len());
    let block = &b[base..end];
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY on each arm: the tier is the CPU's own feature report, and
        // a rung implies every rung below it, so reaching an arm is the
        // guarantee that the features its body was compiled for are present.
        if block.len() == 64 {
            if tier >= crate::isa::Tier::Avx512 {
                return unsafe { pair_lanes_512::<X, Y>(block) };
            }
            if tier >= crate::isa::Tier::Avx2 {
                return unsafe { pair_lanes_avx2::<X, Y>(block) };
            }
        }
    }
    let mut mask = 0u64;
    for (i, &c) in block.iter().enumerate() {
        if c == X || c == Y {
            mask |= 1u64 << i;
        }
    }
    mask
}

/// [`pair_lanes_at`]'s block of exactly sixty-four bytes, where the two
/// compares yield the mask itself.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn pair_lanes_512<const X: u8, const Y: u8>(block: &[u8]) -> u64 {
    use core::arch::x86_64::{_mm512_cmpeq_epi8_mask, _mm512_loadu_si512, _mm512_set1_epi8};
    // SAFETY: the caller hands a block of exactly sixty-four bytes.
    let v = unsafe { _mm512_loadu_si512(block.as_ptr().cast()) };
    _mm512_cmpeq_epi8_mask(v, _mm512_set1_epi8(X as i8)) | _mm512_cmpeq_epi8_mask(v, _mm512_set1_epi8(Y as i8))
}

/// [`pair_lanes_512`] as two thirty-two lane halves.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn pair_lanes_avx2<const X: u8, const Y: u8>(block: &[u8]) -> u64 {
    use core::arch::x86_64::{_mm256_cmpeq_epi8, _mm256_loadu_si256, _mm256_movemask_epi8, _mm256_or_si256, _mm256_set1_epi8};
    let half = |at: usize| {
        // SAFETY: the caller hands a block of exactly sixty-four bytes, so
        // both thirty-two byte loads are inside it.
        let v = unsafe { _mm256_loadu_si256(block.as_ptr().add(at).cast()) };
        let q = _mm256_or_si256(
            _mm256_cmpeq_epi8(v, _mm256_set1_epi8(X as i8)),
            _mm256_cmpeq_epi8(v, _mm256_set1_epi8(Y as i8)),
        );
        _mm256_movemask_epi8(q) as u32 as u64
    };
    half(0) | (half(32) << 32)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn space_run_avx2(b: &[u8]) -> usize {
    use core::arch::x86_64::_mm256_loadu_si256;
    let n = b.len();
    let mut i = 0;
    while i + 32 <= n {
        // SAFETY: the 32-byte load is bounded by `i + 32 <= n`, and the
        // lane test runs under the same feature as this function.
        let mask = unsafe { whitespace_lanes(_mm256_loadu_si256(b.as_ptr().add(i).cast())) };
        if mask != 0xFFFF_FFFF {
            return i + (!mask).trailing_zeros() as usize;
        }
        i += 32;
    }
    i + space_run_scalar(&b[i..])
}

/// [`space_run_avx2`] over 64 bytes a step.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn space_run_avx512(b: &[u8]) -> usize {
    use core::arch::x86_64::_mm512_loadu_si512;
    let n = b.len();
    let mut i = 0;
    while i + 64 <= n {
        // SAFETY: the 64-byte load is bounded by `i + 64 <= n`, and the lane
        // test runs under the same features as this function.
        let mask =
            unsafe { whitespace_lanes_512(_mm512_loadu_si512(b.as_ptr().add(i).cast())) };
        if mask != u64::MAX {
            return i + (!mask).trailing_zeros() as usize;
        }
        i += 64;
    }
    i + space_run_scalar(&b[i..])
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn nonspace_run_avx2(b: &[u8]) -> usize {
    use core::arch::x86_64::_mm256_loadu_si256;
    let n = b.len();
    let mut i = 0;
    while i + 32 <= n {
        // SAFETY: the 32-byte load is bounded by `i + 32 <= n`, and the
        // lane test runs under the same feature as this function.
        let mask = unsafe { whitespace_lanes(_mm256_loadu_si256(b.as_ptr().add(i).cast())) };
        if mask != 0 {
            return i + mask.trailing_zeros() as usize;
        }
        i += 32;
    }
    i + nonspace_run_scalar(&b[i..])
}

/// [`nonspace_run_avx2`] over 64 bytes a step.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn nonspace_run_avx512(b: &[u8]) -> usize {
    use core::arch::x86_64::_mm512_loadu_si512;
    let n = b.len();
    let mut i = 0;
    while i + 64 <= n {
        // SAFETY: the 64-byte load is bounded by `i + 64 <= n`, and the lane
        // test runs under the same features as this function.
        let mask =
            unsafe { whitespace_lanes_512(_mm512_loadu_si512(b.as_ptr().add(i).cast())) };
        if mask != 0 {
            return i + mask.trailing_zeros() as usize;
        }
        i += 64;
    }
    i + nonspace_run_scalar(&b[i..])
}

/// Length of the leading run of ASCII digit bytes (`'0'..='9'`). The lexer
/// scans the integer and fractional parts of a number this way.
#[must_use]
#[inline]
pub fn digit_run(b: &[u8]) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY on each arm: the tier is resolved once per process from the
        // CPU's own feature report, and a rung implies every rung below it,
        // so reaching an arm is the guarantee that the features its body was
        // compiled for are present.
        if crate::isa::tier() >= crate::isa::Tier::Avx512 {
            return unsafe { digit_run_avx512(b) };
        }
        if crate::isa::tier() >= crate::isa::Tier::Avx2 {
            return unsafe { digit_run_avx2(b) };
        }
    }
    digit_run_scalar(b)
}

/// Scalar reference for [`digit_run`].
#[must_use]
pub fn digit_run_scalar(b: &[u8]) -> usize {
    b.iter().take_while(|c| c.is_ascii_digit()).count()
}

/// Offset of the first ASCII digit byte in `b`, or `None` where it holds none.
///
/// The mirror of [`digit_run`], which reports where a run of digits stops: the
/// same compare pair, read for the first bit set rather than the first clear. A
/// byte route looking for where a number token could begin reads the input this
/// way rather than one byte at a time.
#[must_use]
#[inline]
pub fn digit_find(b: &[u8]) -> Option<usize> {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY on each arm: the tier is resolved once per process from the
        // CPU's own feature report, and a rung implies every rung below it, so
        // reaching an arm is the guarantee that the features its body was
        // compiled for are present.
        if crate::isa::tier() >= crate::isa::Tier::Avx512 {
            return unsafe { digit_find_avx512(b) };
        }
        if crate::isa::tier() >= crate::isa::Tier::Avx2 {
            return unsafe { digit_find_avx2(b) };
        }
    }
    digit_find_scalar(b)
}

/// Scalar reference for [`digit_find`].
#[must_use]
pub fn digit_find_scalar(b: &[u8]) -> Option<usize> {
    b.iter().position(u8::is_ascii_digit)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn digit_find_avx2(b: &[u8]) -> Option<usize> {
    use core::arch::x86_64::{
        _mm256_and_si256, _mm256_cmpgt_epi8, _mm256_loadu_si256, _mm256_movemask_epi8,
        _mm256_set1_epi8,
    };
    let n = b.len();
    let mut i = 0;
    while i + 32 <= n {
        // SAFETY: the 32-byte load is bounded by `i + 32 <= n`.
        let v = unsafe { _mm256_loadu_si256(b.as_ptr().add(i).cast()) };
        // `'0'..='9'` is the signed range `b > 47 && 58 > b`.
        let digit = _mm256_and_si256(
            _mm256_cmpgt_epi8(v, _mm256_set1_epi8(47)),
            _mm256_cmpgt_epi8(_mm256_set1_epi8(58), v),
        );
        let mask = _mm256_movemask_epi8(digit) as u32;
        if mask != 0 {
            return Some(i + mask.trailing_zeros() as usize);
        }
        i += 32;
    }
    digit_find_scalar(&b[i..]).map(|k| i + k)
}

/// [`digit_find_avx2`] over 64 bytes a step.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn digit_find_avx512(b: &[u8]) -> Option<usize> {
    use core::arch::x86_64::{_mm512_cmpgt_epi8_mask, _mm512_loadu_si512, _mm512_set1_epi8};
    let n = b.len();
    let mut i = 0;
    while i + 64 <= n {
        // SAFETY: the 64-byte load is bounded by `i + 64 <= n`.
        let v = unsafe { _mm512_loadu_si512(b.as_ptr().add(i).cast()) };
        // `'0'..='9'` is the signed range `b > 47 && 58 > b`.
        let digit: u64 = _mm512_cmpgt_epi8_mask(v, _mm512_set1_epi8(47))
            & _mm512_cmpgt_epi8_mask(_mm512_set1_epi8(58), v);
        if digit != 0 {
            return Some(i + digit.trailing_zeros() as usize);
        }
        i += 64;
    }
    digit_find_scalar(&b[i..]).map(|k| i + k)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn digit_run_avx2(b: &[u8]) -> usize {
    use core::arch::x86_64::{
        _mm256_and_si256, _mm256_cmpgt_epi8, _mm256_loadu_si256, _mm256_movemask_epi8,
        _mm256_set1_epi8,
    };
    let n = b.len();
    let mut i = 0;
    while i + 32 <= n {
        // SAFETY: the 32-byte load is bounded by `i + 32 <= n`.
        let v = unsafe { _mm256_loadu_si256(b.as_ptr().add(i).cast()) };
        // `'0'..='9'` is the signed range `b > 47 && 58 > b`.
        let digit = _mm256_and_si256(
            _mm256_cmpgt_epi8(v, _mm256_set1_epi8(47)),
            _mm256_cmpgt_epi8(_mm256_set1_epi8(58), v),
        );
        let mask = _mm256_movemask_epi8(digit) as u32;
        if mask != 0xFFFF_FFFF {
            return i + (!mask).trailing_zeros() as usize;
        }
        i += 32;
    }
    i + digit_run_scalar(&b[i..])
}

/// [`digit_run_avx2`] over 64 bytes a step.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f,avx512bw")]
unsafe fn digit_run_avx512(b: &[u8]) -> usize {
    use core::arch::x86_64::{_mm512_cmpgt_epi8_mask, _mm512_loadu_si512, _mm512_set1_epi8};
    let n = b.len();
    let mut i = 0;
    while i + 64 <= n {
        // SAFETY: the 64-byte load is bounded by `i + 64 <= n`.
        let v = unsafe { _mm512_loadu_si512(b.as_ptr().add(i).cast()) };
        // `'0'..='9'` is the signed range `b > 47 && 58 > b`.
        let digit: u64 = _mm512_cmpgt_epi8_mask(v, _mm512_set1_epi8(47))
            & _mm512_cmpgt_epi8_mask(_mm512_set1_epi8(58), v);
        if digit != u64::MAX {
            return i + (!digit).trailing_zeros() as usize;
        }
        i += 64;
    }
    i + digit_run_scalar(&b[i..])
}

/// A byte class compiled to the pair of sixteen-entry tables a shuffle
/// instruction indexes, so membership is answered thirty-two bytes at a time.
///
/// `vpshufb` looks up thirty-two bytes against a sixteen-entry table in one
/// instruction, and sixteen entries is four bits of index, so a byte's two
/// nibbles are two lookups and its membership is their meet. `lo[l]` carries a
/// bit for each high nibble `h` under eight such that the byte `h << 4 | l` is
/// in the class, and `hi[h]` carries that one bit.
///
/// Distinct from the searches above in what it answers and therefore in what
/// it costs. [`find`] locates a byte string; this reports which bytes belong
/// to a set, which is what a run-length question is asked in terms of - a
/// base64 blob is a run of at least sixteen base64 bytes, a hash digest a hex
/// run of exactly thirty-two, forty or sixty-four. Those are necessary
/// conditions for their kinds, so an input holding no long enough run holds no
/// token of the kind, and a scan can answer without lexing.
///
/// It carries no state between bytes, which is what separates it from
/// [`crate::byte_dfa::ByteDfa`]: that steps a table with the value it just
/// loaded, a dependent load chain that cannot vectorize however it is written.
#[derive(Clone, Copy, Debug)]
pub struct ClassTables {
    lo: [u8; 16],
    hi: [u8; 16],
}

impl ClassTables {
    /// The tables for `class`, or `None` where it holds a byte at or above
    /// `0x80`.
    ///
    /// One pair of tables spells the high nibbles nought to seven, so a class
    /// reaching above ASCII cannot be answered by it. Refused rather than
    /// answered over its lower half, because a filter that silently drops the
    /// upper half of a class refuses inputs that match.
    #[must_use]
    pub fn for_class(class: &crate::byte_nfa::ByteClass) -> Option<ClassTables> {
        let mut lo = [0u8; 16];
        for b in 0..=255u8 {
            if !class.has(b) {
                continue;
            }
            if b >= 0x80 {
                return None;
            }
            lo[(b & 0x0F) as usize] |= 1 << (b >> 4);
        }
        let mut hi = [0u8; 16];
        for (h, slot) in hi.iter_mut().enumerate() {
            *slot = if h < 8 { 1 << h } else { 0 };
        }
        Some(ClassTables { lo, hi })
    }

    /// Whether `b` is in the class these tables describe.
    #[must_use]
    pub fn has(&self, b: u8) -> bool {
        self.lo[(b & 0x0F) as usize] & self.hi[((b >> 4) & 0x0F) as usize] != 0
    }

    /// Whether `input` holds a run of at least `least` class bytes.
    ///
    /// What a necessary condition actually asks, and it stops at the first run
    /// that reaches `least` rather than reading to the end. The saving is
    /// entirely in the case where the answer is yes: an input that holds the
    /// run is answered where the run is, and one that does not is read in full
    /// either way, because absence is not knowable from a prefix.
    #[must_use]
    pub fn has_run_of(&self, input: &[u8], least: usize) -> bool {
        if least == 0 {
            return true;
        }
        self.run_scan(input, Some(least)) >= least
    }

    /// The longest run of class bytes in `input`.
    ///
    /// The question a necessary condition is asked in: a kind whose shortest
    /// token is `n` bytes of this class cannot occur in an input whose longest
    /// run is shorter.
    #[must_use]
    pub fn longest_run(&self, input: &[u8]) -> usize {
        self.run_scan(input, None)
    }

    /// The longest run of class bytes in `input`, stopping once one reaches
    /// `stop_at` where a bound is given.
    ///
    /// One implementation for both questions, so the bounded form cannot drift
    /// from the unbounded one. A bounded scan's answer is the longest run it
    /// saw, which is at least `stop_at` when it stopped early and the true
    /// longest when it did not - enough for a threshold test and not to be
    /// read as a maximum.
    fn run_scan(&self, input: &[u8], stop_at: Option<usize>) -> usize {
        #[cfg(target_arch = "x86_64")]
        {
            // SAFETY: as every arm above - `crate::isa::tier` reports the rungs
            // this CPU can run, resolved once for the process, and a rung
            // implies every rung below it.
            if matches!(crate::isa::tier(), crate::isa::Tier::Avx2 | crate::isa::Tier::Avx512) {
                return unsafe { self.run_scan_avx2(input, stop_at) };
            }
        }
        self.run_scan_scalar(input, stop_at)
    }

    /// [`ClassTables::longest_run`] a byte at a time, which is what a target
    /// without AVX2 runs and what the vector path is checked against.
    #[must_use]
    pub fn longest_run_scalar(&self, input: &[u8]) -> usize {
        self.run_scan_scalar(input, None)
    }

    /// [`ClassTables::run_scan`] a byte at a time.
    fn run_scan_scalar(&self, input: &[u8], stop_at: Option<usize>) -> usize {
        let mut longest = 0usize;
        let mut run = 0usize;
        for &b in input {
            if self.has(b) {
                run += 1;
                if run > longest {
                    longest = run;
                    if stop_at.is_some_and(|n| longest >= n) {
                        return longest;
                    }
                }
            } else {
                run = 0;
            }
        }
        longest
    }

    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx2")]
    unsafe fn run_scan_avx2(&self, input: &[u8], stop_at: Option<usize>) -> usize {
        use core::arch::x86_64::{
            __m256i, _mm256_and_si256, _mm256_broadcastsi128_si256, _mm256_cmpeq_epi8,
            _mm256_loadu_si256, _mm256_movemask_epi8, _mm256_set1_epi8, _mm256_setzero_si256,
            _mm256_shuffle_epi8, _mm256_srli_epi16, _mm_loadu_si128,
        };

        // SAFETY: both loads are of sixteen bytes from a sixteen-byte array.
        let lo_tbl =
            _mm256_broadcastsi128_si256(unsafe { _mm_loadu_si128(self.lo.as_ptr().cast()) });
        let hi_tbl =
            _mm256_broadcastsi128_si256(unsafe { _mm_loadu_si128(self.hi.as_ptr().cast()) });
        let low_nibble = _mm256_set1_epi8(0x0F);
        let zero = _mm256_setzero_si256();

        let mut longest = 0usize;
        let mut run = 0usize;
        let mut at = 0usize;

        while at + 32 <= input.len() {
            // SAFETY: the 32-byte load is bounded by `at + 32 <= input.len()`.
            let v: __m256i = unsafe { _mm256_loadu_si256(input.as_ptr().add(at).cast()) };
            let lo_idx = _mm256_and_si256(v, low_nibble);
            // `srli_epi16` shifts sixteen bits at a time, so the byte below
            // each pair brings its top four bits down into this one; the mask
            // removes them and leaves the high nibble alone.
            let hi_idx = _mm256_and_si256(_mm256_srli_epi16::<4>(v), low_nibble);
            let met = _mm256_and_si256(
                _mm256_shuffle_epi8(lo_tbl, lo_idx),
                _mm256_shuffle_epi8(hi_tbl, hi_idx),
            );
            let absent = _mm256_movemask_epi8(_mm256_cmpeq_epi8(met, zero)) as u32;
            let present = !absent;

            // Bit `i` is byte `i`, so a run has three parts: the ones at the
            // bottom continue the run carried in, the ones at the top become
            // the run carried out, and every run between them is whole inside
            // this word. A word that is all ones is none of the three and
            // extends the carry.
            if present == u32::MAX {
                run += 32;
                if run > longest {
                    longest = run;
                }
            } else {
                let head = present.trailing_ones() as usize;
                run += head;
                if run > longest {
                    longest = run;
                }
                let tail = present.leading_ones() as usize;
                let mut rest = present >> head;
                let mut seen = head;
                while rest != 0 {
                    let gap = rest.trailing_zeros() as usize;
                    rest >>= gap;
                    seen += gap;
                    if rest == 0 {
                        break;
                    }
                    let ones = rest.trailing_ones() as usize;
                    // A run reaching the top of the word is the tail, counted
                    // once below as the carry rather than twice here.
                    if seen + ones < 32 && ones > longest {
                        longest = ones;
                    }
                    rest >>= ones;
                    seen += ones;
                }
                run = tail;
                if run > longest {
                    longest = run;
                }
            }
            // Checked a vector at a time rather than at each of the four
            // places above: the answer a bounded caller wants is a threshold
            // test, and thirty-two bytes of extra reading cannot change it.
            if stop_at.is_some_and(|n| longest >= n) {
                return longest;
            }
            at += 32;
        }

        // The tail, by the same rule as the body so a run crossing into it is
        // one run and not two.
        for &b in &input[at..] {
            if self.has(b) {
                run += 1;
                if run > longest {
                    longest = run;
                }
            } else {
                run = 0;
            }
        }
        longest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every quote of this file's own bytes, asked for from the quote before
    /// it and from positions jumped past the way a scan over strings jumps
    /// them - by strides inside a block, across blocks and over many - is the
    /// quote a byte-by-byte reading finds, and nothing is left past the end.
    /// The same holds of the double quotes and newlines a string's close is
    /// read off, and of asks that move back, as a scan's do when it visits the
    /// quotes inside a string whose close it has already read.
    #[test]
    fn pair_positions_are_the_bytes_a_reading_finds() {
        fn check<const X: u8, const Y: u8>(text: &[u8]) {
            let want: Vec<usize> =
                text.iter().enumerate().filter(|&(_, &c)| c == X || c == Y).map(|(i, _)| i).collect();
            assert!(want.len() > 100, "the file holds the bytes to find");
            let first_at_or_after = |from: usize| want[want.partition_point(|&q| q < from)..].first().copied();
            for stride in [1usize, 2, 5, 63, 64, 65, 200, 4097] {
                let mut found = PairPositions::<'_, X, Y>::new(text);
                let mut from = 0;
                let mut handed = 0;
                while let Some(p) = found.next_at_or_after(from) {
                    assert_eq!(Some(p), first_at_or_after(from), "from {from}, stride {stride}");
                    handed += 1;
                    from = p + stride;
                }
                assert_eq!(first_at_or_after(from), None, "stride {stride}: a byte at or past {from} was not handed out");
                if stride == 1 {
                    assert_eq!(handed, want.len());
                }
            }
            // From the end of the text back to its start, then back and forth
            // across the first blocks' edges.
            let mut found = PairPositions::<'_, X, Y>::new(text);
            for from in (0..text.len()).rev().step_by(37).chain([0, 130, 1, 64, 63, 5000, 64, 0]) {
                assert_eq!(found.next_at_or_after(from), first_at_or_after(from), "from {from}, asked out of order");
            }
            assert_eq!(found.next_at_or_after(text.len()), None);
            assert_eq!(found.next_at_or_after(text.len() + 100), None);
        }
        let text: &[u8] = include_bytes!("byte_simd.rs");
        check::<b'"', b'\''>(text);
        check::<b'"', b'\n'>(text);
        assert_eq!(QuotePositions::new(b"").next_at_or_after(0), None);
        assert_eq!(QuotePositions::new(b"no quote here").next_at_or_after(0), None);
        assert_eq!(CloseOrNewlinePositions::new(b"no close here").next_at_or_after(0), None);
    }

    #[test]
    fn digit_find_matches_scalar() {
        // The tier arms are what this holds: a haystack crossing many vector
        // windows and holding bytes >= 128, read for the first digit rather
        // than the end of a run, so a mask taken for the wrong bit shows here.
        let big: Vec<u8> = (0..6000u32)
            .map(|i| {
                let r = i.wrapping_mul(2_654_435_761) >> 23;
                match r % 5 {
                    0 => (r & 0xFF) as u8,
                    1 => b'0' + (r % 10) as u8,
                    2 => b' ',
                    _ => b'a' + (r % 26) as u8,
                }
            })
            .collect();
        for start in [0usize, 1, 31, 32, 33, 63, 64, 65, 1000, 5999] {
            let tail = &big[start.min(big.len())..];
            assert_eq!(digit_find(tail), digit_find_scalar(tail), "from {start}");
        }
        // The edges of the vector loop: nothing at all, no digit anywhere, one
        // in the first lane, one in the last lane of a full 64-byte block, and
        // one past every full block so only the scalar tail finds it.
        assert_eq!(digit_find(b""), None);
        assert_eq!(digit_find(&[b'x'; 200]), None);
        let mut first = [b'x'; 200];
        first[0] = b'7';
        assert_eq!(digit_find(&first), Some(0));
        let mut block = [b'x'; 200];
        block[63] = b'7';
        assert_eq!(digit_find(&block), Some(63));
        let mut tail = [b'x'; 200];
        tail[199] = b'7';
        assert_eq!(digit_find(&tail), Some(199));
    }

    #[test]
    fn run_finders_match_scalar() {
        // A haystack crossing many 32-byte windows, including bytes >= 128.
        let big: Vec<u8> = (0..6000u32)
            .map(|i| {
                let r = i.wrapping_mul(2_654_435_761) >> 23;
                if r % 4 == 0 {
                    (r & 0xFF) as u8
                } else {
                    b" \t\n0189abcXYZ_"[(r % 13) as usize]
                }
            })
            .collect();
        for start in [0usize, 1, 5, 31, 32, 33, 63, 64, 100, 1000, 5990, 5999] {
            let s = &big[start..];
            assert_eq!(space_run(s), space_run_scalar(s), "space @ {start}");
            assert_eq!(digit_run(s), digit_run_scalar(s), "digit @ {start}");
        }
        assert_eq!(space_run(b"   \t\nx"), 5);
        assert_eq!(space_run(b"\x0b spaces"), 0); // vertical tab is not whitespace
        assert_eq!(digit_run(b"0123456789012345678901234567890123!"), 34);
        assert_eq!(digit_run(b"12.5"), 2);
    }

    #[test]
    fn word_run_matches_scalar() {
        // A haystack that crosses many 32-byte windows and mixes word and
        // non-word bytes, including bytes >= 128 (non-word, negative i8).
        let big: Vec<u8> = (0..5000u32)
            .map(|i| {
                let r = i.wrapping_mul(2_654_435_761) >> 24;
                // Bias toward word bytes so runs span SIMD windows.
                if r % 5 == 0 { (r & 0xFF) as u8 } else { b"abcXYZ_0189"[(r % 11) as usize] }
            })
            .collect();
        for start in [0usize, 1, 5, 31, 32, 33, 63, 64, 100, 1000, 4990, 4999] {
            assert_eq!(
                word_run(&big[start..]),
                word_run_scalar(&big[start..]),
                "mismatch at start {start}"
            );
        }
        // Edge cases: empty, immediate non-word, and a run ending exactly on
        // each window width so the vector loop and its tail both decide one.
        assert_eq!(word_run(b""), 0);
        assert_eq!(word_run(b" abc"), 0);
        assert_eq!(word_run(b"abcdefghijklmnopqrstuvwxyz012345!"), 32);
        assert_eq!(word_run(b"name_1 rest"), 6);
        for len in [31usize, 32, 33, 63, 64, 65, 127, 128, 129] {
            let mut run: Vec<u8> = std::iter::repeat_n(b'w', len).collect();
            run.push(b' ');
            run.extend_from_slice(b"tail");
            assert_eq!(word_run(&run), len, "a run of {len} word bytes");
            assert_eq!(word_run_scalar(&run), len, "the scalar reference at {len}");
        }
    }

    #[test]
    fn every_run_function_agrees_with_its_reference_at_both_window_widths() {
        // A run ending exactly on each width, and one byte either side of it,
        // so the vector loop decides some and its tail decides others on a
        // 32-byte step and on a 64-byte one.
        for len in [0usize, 1, 31, 32, 33, 63, 64, 65, 127, 128, 129, 200] {
            for (class, other) in [(b'w', b' '), (b'7', b'x'), (b' ', b'w'), (b'x', b'\n')] {
                let mut run: Vec<u8> = std::iter::repeat_n(class, len).collect();
                run.push(other);
                run.extend_from_slice(b"tail rest");
                assert_eq!(word_run(&run), word_run_scalar(&run), "word at {len}");
                assert_eq!(digit_run(&run), digit_run_scalar(&run), "digit at {len}");
                assert_eq!(space_run(&run), space_run_scalar(&run), "space at {len}");
                assert_eq!(
                    nonspace_run(&run),
                    nonspace_run_scalar(&run),
                    "nonspace at {len}"
                );
            }
        }
    }

    #[test]
    fn nonspace_run_matches_scalar() {
        // Whitespace bytes and the near miss (11, vertical tab, is not one)
        // scattered through long runs of everything else, so the SIMD
        // windows hold both and every whitespace byte ends a run where the
        // scalar reference says.
        let big: Vec<u8> = (0..5000u32)
            .map(|i| {
                let r = i.wrapping_mul(2_654_435_761) >> 24;
                match r % 23 {
                    0 => b' ',
                    1 => b'\n',
                    2 => b'\t',
                    3 => 11,
                    _ => b"abcXYZ_0189+/=."[(r % 15) as usize],
                }
            })
            .collect();
        for start in [0usize, 1, 5, 31, 32, 33, 63, 64, 100, 1000, 4990, 4999] {
            assert_eq!(
                nonspace_run(&big[start..]),
                nonspace_run_scalar(&big[start..]),
                "mismatch at start {start}"
            );
        }
        assert_eq!(nonspace_run(b""), 0);
        assert_eq!(nonspace_run(b" abc"), 0);
        assert_eq!(nonspace_run(b"abcdefghijklmnopqrstuvwxyz012345 "), 32);
        assert_eq!(nonspace_run(b"name_1\x0bx rest"), 8);
        assert_eq!(nonspace_run(b"\x0c"), 0);
    }

    #[test]
    fn nonspace_spans_match_scalar() {
        // Whitespace at chunk edges, runs crossing chunks, runs at both ends
        // and none at all; every min_len around the chunk width, where the
        // SIMD path changes shape, agrees with the scalar reference.
        let big: Vec<u8> = (0..6000u32)
            .map(|i| {
                let r = i.wrapping_mul(2_654_435_761) >> 24;
                if r % 41 == 0 || i % 1000 == 31 || i % 1000 == 32 {
                    b' '
                } else {
                    b"abcXYZ_0189+/=."[(r % 15) as usize]
                }
            })
            .collect();
        // The ladder crosses both chunk widths, 32 and 64, because each SIMD
        // path changes shape where `min_len` reaches its own.
        for min_len in [1usize, 2, 16, 31, 32, 33, 48, 63, 64, 65, 100, 1000] {
            for start in [0usize, 1, 31, 32, 33, 63, 64, 65, 100, 127, 128, 5990] {
                assert_eq!(
                    nonspace_spans_at_least(&big[start..], min_len),
                    nonspace_spans_at_least_scalar(&big[start..], min_len),
                    "min_len {min_len} from {start}"
                );
            }
        }
        assert_eq!(nonspace_spans_at_least(b"", 1), Vec::new());
        assert_eq!(nonspace_spans_at_least(b"   ", 1), Vec::new());
        assert_eq!(nonspace_spans_at_least(b"abc", 3), vec![(0, 3)]);
        assert_eq!(nonspace_spans_at_least(b"abc def", 3), vec![(0, 3), (4, 7)]);
        assert_eq!(nonspace_spans_at_least(b"ab cdef", 3), vec![(3, 7)]);
        let long: Vec<u8> =
            std::iter::repeat_n(b'x', 70).chain(*b"\n").chain(std::iter::repeat_n(b'y', 47)).collect();
        assert_eq!(nonspace_spans_at_least(&long, 48), vec![(0, 70)]);
    }

    #[test]
    fn dispatched_find_matches_scalar_on_small_cases() {
        let cases: &[(&[u8], &[u8])] = &[
            (b"hello world", b"world"),
            (b"hello world", b"xyz"),
            (b"aaaaaaab", b"ab"),
            (b"abcabcabc", b"cab"),
            (b"", b"x"),
            (b"x", b""),
            (b"needle at end NEEDLE", b"NEEDLE"),
            (b"\x00\x01\x02\x03", b"\x02\x03"),
        ];
        for &(h, n) in cases {
            assert_eq!(find(h, n), find_scalar(h, n), "h={h:?} n={n:?}");
        }
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn each_simd_path_matches_scalar_on_large_input() {
        // A long pseudo-random haystack that crosses many SIMD chunks.
        let big: Vec<u8> =
            (0..5000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
        let needles: Vec<&[u8]> = vec![
            &big[0..1],
            &big[100..104],
            &big[2500..2506],
            &big[4996..5000], // only at the very end, past the last window
            &big[1000..1100], // longer than the window the probe reads
            b"\xff\xff\xff",
            b"\xff\xff", // first byte and last byte the same
        ];
        if std::is_x86_feature_detected!("avx2") {
            for nd in &needles {
                assert_eq!(unsafe { find_avx2(&big, nd) }, find_scalar(&big, nd));
            }
        }
        if std::is_x86_feature_detected!("sse2") {
            for nd in &needles {
                assert_eq!(unsafe { find_sse2(&big, nd) }, find_scalar(&big, nd));
            }
        }
        // The real AVX-512 path, exercised only where the hardware has it.
        #[cfg(target_arch = "x86_64")]
        if std::is_x86_feature_detected!("avx512f") && std::is_x86_feature_detected!("avx512bw") {
            for nd in &needles {
                assert_eq!(unsafe { find_avx512(&big, nd) }, find_scalar(&big, nd));
            }
        }
    }

    /// The counting searches answer what their scalar baselines answer: the
    /// `n`th occurrence from each end at every `n` from zero (none) to one past
    /// the last (none), and the count, one pass and split, over a haystack that
    /// crosses many blocks and ends partway into one, and over inputs short
    /// enough to be all tail.
    #[test]
    fn the_counting_searches_answer_what_the_scalar_baselines_answer() {
        let big: Vec<u8> = (0..5003u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
        let texts: Vec<&[u8]> = vec![&big, &big[..1], &big[..15], &big[..31], &big[..63], &big[..65], b"", b"\n\n\n"];
        for text in &texts {
            for byte in [big[0], b'\n', 0xFF] {
                let total = count_byte_scalar(text, byte);
                assert_eq!(count_byte(text, byte), total, "count of {byte} over {} bytes", text.len());
                assert_eq!(count_byte_across(text, byte), total, "split count of {byte} over {} bytes", text.len());
                for n in 0..=total + 1 {
                    assert_eq!(nth_byte(text, byte, n), nth_byte_scalar(text, byte, n), "{n}th {byte}");
                    assert_eq!(nth_byte_back(text, byte, n), nth_byte_back_scalar(text, byte, n), "{n}th {byte} from the end");
                }
            }
        }
    }

    /// Each vector path of the counting searches, called by name where this CPU
    /// has its features, answers what the scalar baseline answers.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn each_counting_path_matches_scalar() {
        let big: Vec<u8> = (0..5003u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
        let byte = big[7];
        let total = count_byte_scalar(&big, byte);
        let check = |name: &str,
                     nth: &dyn Fn(usize) -> Option<usize>,
                     back: &dyn Fn(usize) -> Option<usize>,
                     count: &dyn Fn() -> usize| {
            assert_eq!(count(), total, "{name} count");
            // From one: a count of zero is answered by the dispatch, before a
            // vector path is chosen.
            for n in 1..=total + 1 {
                assert_eq!(nth(n), nth_byte_scalar(&big, byte, n), "{name} {n}th");
                assert_eq!(back(n), nth_byte_back_scalar(&big, byte, n), "{name} {n}th from the end");
            }
        };
        if std::is_x86_feature_detected!("sse2") {
            // SAFETY: each call is behind the detection of the feature it needs.
            check(
                "sse2",
                &|n| unsafe { nth_byte_sse2(&big, byte, n) },
                &|n| unsafe { nth_byte_back_sse2(&big, byte, n) },
                &|| unsafe { count_byte_sse2(&big, byte) },
            );
        }
        if std::is_x86_feature_detected!("avx2") {
            // SAFETY: as above.
            check(
                "avx2",
                &|n| unsafe { nth_byte_avx2(&big, byte, n) },
                &|n| unsafe { nth_byte_back_avx2(&big, byte, n) },
                &|| unsafe { count_byte_avx2(&big, byte) },
            );
        }
        if std::is_x86_feature_detected!("avx512f") && std::is_x86_feature_detected!("avx512bw") {
            // SAFETY: as above.
            check(
                "avx512",
                &|n| unsafe { nth_byte_avx512(&big, byte, n) },
                &|n| unsafe { nth_byte_back_avx512(&big, byte, n) },
                &|| unsafe { count_byte_avx512(&big, byte) },
            );
        }
    }

    #[test]
    fn avx512_algorithm_is_byte_exact_via_emulation() {
        // The scalar emulation of the 64-wide AVX-512 algorithm must
        // equal the scalar baseline on every case, which proves the
        // AVX-512 logic is byte-exact on a host that cannot run the real
        // instructions.
        let big: Vec<u8> =
            (0..5000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
        let needles: Vec<&[u8]> = vec![
            &big[0..1],
            &big[63..67],     // a needle straddling the 64-byte window edge
            &big[64..68],     // a needle at the next window's start
            &big[100..104],
            &big[2500..2506],
            &big[4996..5000], // only at the very end, past the last window
            &big[1000..1100], // longer than the window the probe reads
            b"\xff\xff\xff",
            b"\xff\xff",      // first byte and last byte the same
            b"",
        ];
        for nd in &needles {
            assert_eq!(
                find_avx512_emulated(&big, nd),
                find_scalar(&big, nd),
                "emulated AVX-512 disagrees with scalar on needle {nd:?}"
            );
        }
        // Also the small/edge cases the dispatcher table covers.
        let cases: &[(&[u8], &[u8])] = &[
            (b"hello world", b"world"),
            (b"hello world", b"xyz"),
            (b"", b"x"),
            (b"x", b""),
            (b"\x00\x01\x02\x03", b"\x02\x03"),
        ];
        for &(h, n) in cases {
            assert_eq!(find_avx512_emulated(h, n), find_scalar(h, n), "h={h:?} n={n:?}");
        }
    }

    /// The split search reports what the one-pass search reports, in every
    /// case a boundary makes: an occurrence straddling one, two
    /// occurrences overlapping across one, a needle longer than a leaf's
    /// slice, and a needle that occurs nowhere.
    #[test]
    fn the_split_find_all_reports_what_one_pass_reports() {
        let mut hay = Vec::new();
        for i in 0..20_000u32 {
            hay.extend_from_slice(format!("let value_{i} = {i} ; call_{i}(alpha) ;\n").as_bytes());
        }
        // Overlapping occurrences, so a leaf that reported only the first per
        // window would differ; and a run of them long enough to cross a leaf.
        hay.extend_from_slice(&b"aaaaaaaaaaaaaaaaaaaaaaaa\n".repeat(400));
        for needle in [
            &b"let"[..],
            &b"alpha"[..],
            &b"aa"[..],
            &b"aaaa"[..],
            &b"zzzqqq"[..],
            &b"\n"[..],
            &b"value_19999 = 19999"[..],
        ] {
            assert_eq!(
                find_all_across(&hay, needle),
                find_all_one_pass(&hay, needle),
                "needle {needle:?} over {} bytes",
                hay.len()
            );
        }
        // Inputs shorter than one leaf, and the refusals.
        for n in [0usize, 1, 2, 3, 64, 4096] {
            let small = &hay[..n.min(hay.len())];
            assert_eq!(find_all_across(small, b"let"), find_all(small, b"let"), "{n} bytes");
        }
        assert!(find_all_across(&hay, b"").is_empty(), "an empty needle reports nothing");
        assert!(find_all(&hay, b"").is_empty(), "and the one-pass form agrees");
    }

    /// The spanned walk reports what the one-pass search reports, over spans
    /// small enough that the boundary between two of them is at every place it
    /// can be: inside an occurrence, between two overlapping ones, and on a span
    /// whose search finds nothing at all.
    ///
    /// The span is named here rather than taken from the constant, because the
    /// real one is eight megabytes and every case below would need an input
    /// that size to reach a boundary.
    #[test]
    fn the_spanned_walk_reports_what_one_pass_reports() {
        let mut hay = Vec::new();
        for i in 0..600u32 {
            hay.extend_from_slice(format!("let value_{i} = {i} ; call_{i}(alpha) ;\n").as_bytes());
        }
        hay.extend_from_slice(&b"aaaaaaaaaaaaaaaaaaaaaaaa\n".repeat(40));
        for needle in [
            &b"let"[..],
            &b"alpha"[..],
            &b"aa"[..],
            &b"aaaa"[..],
            &b"zzzqqq"[..],
            &b"\n"[..],
            &b"a"[..],
        ] {
            let want = find_all_one_pass(&hay, needle);
            // Spans down to one byte, so a boundary is at every position in
            // turn and a needle longer than a whole span is covered too.
            for span in [1usize, 2, 3, 7, 64, 4096, hay.len(), hay.len() * 2] {
                let got: Vec<usize> = occurrences_by_span(&hay, needle, span).collect();
                assert_eq!(got, want, "needle {needle:?} at span {span}");
            }
        }
        // The refusals, and an input shorter than one span.
        for span in [1usize, 8, 4096] {
            assert!(occurrences_by_span(&hay, b"", span).next().is_none(), "an empty needle");
            for n in [0usize, 1, 2, 3, 64] {
                let small = &hay[..n.min(hay.len())];
                let got: Vec<usize> = occurrences_by_span(small, b"let", span).collect();
                assert_eq!(got, find_all_one_pass(small, b"let"), "{n} bytes at span {span}");
            }
        }
        // A span of zero is taken as one rather than looping forever.
        let got: Vec<usize> = occurrences(&hay, b"let").collect();
        assert_eq!(got, find_all_one_pass(&hay, b"let"), "the shipped span");
        let got: Vec<usize> = occurrences_by_span(&hay, b"let", 0).collect();
        assert_eq!(got, find_all_one_pass(&hay, b"let"), "a span of zero");
    }

    /// The three forms report the same positions.
    ///
    /// Two of them exist to be timed against each other inside one process, and
    /// that reading is worth nothing unless they answer alike: equal cost is the
    /// question, equal answers is the precondition. A needle that overlaps
    /// itself is the case that separates a walk resuming a byte past each hit
    /// from one resuming past the whole needle.
    #[test]
    fn the_split_walk_and_the_unsplit_walk_report_the_same_positions() {
        let mut hay = Vec::new();
        for i in 0..900u32 {
            hay.extend_from_slice(format!("let value_{i} = {i} ; call_{i}(alpha) ;\n").as_bytes());
        }
        hay.extend_from_slice(&b"aaaaaaaaaaaaaaaaaaaaaaaa\n".repeat(60));
        for needle in [
            &b"let"[..],
            &b"alpha"[..],
            &b"aa"[..],
            &b"aaaa"[..],
            &b"a"[..],
            &b"zzzqqq"[..],
            &b"\n"[..],
            &b""[..],
        ] {
            let one = find_all_one_pass(&hay, needle);
            let spanned: Vec<usize> = occurrences(&hay, needle).collect();
            let unsplit: Vec<usize> = occurrences_unsplit(&hay, needle).collect();
            assert_eq!(spanned, one, "the span walk against one pass, needle {needle:?}");
            assert_eq!(unsplit, one, "the unsplit walk against one pass, needle {needle:?}");
        }
        // An input shorter than the needle, and an empty one, through both.
        for n in [0usize, 1, 2, 3] {
            let small = &hay[..n];
            let spanned: Vec<usize> = occurrences(small, b"let").collect();
            let unsplit: Vec<usize> = occurrences_unsplit(small, b"let").collect();
            assert_eq!(spanned, find_all_one_pass(small, b"let"), "{n} bytes, span walk");
            assert_eq!(unsplit, find_all_one_pass(small, b"let"), "{n} bytes, unsplit walk");
        }
    }

    /// The classes a necessary-condition refusal is asked in terms of.
    fn classes() -> Vec<(&'static str, crate::byte_nfa::ByteClass)> {
        use crate::byte_nfa::ByteClass;
        let digits = ByteClass::range(b'0', b'9');
        let lower = ByteClass::range(b'a', b'z');
        let upper = ByteClass::range(b'A', b'Z');
        vec![
            ("digits", digits),
            ("hex", digits.union(ByteClass::range(b'a', b'f')).union(ByteClass::range(b'A', b'F'))),
            (
                "base64",
                digits
                    .union(lower)
                    .union(upper)
                    .union(ByteClass::just(b'+'))
                    .union(ByteClass::just(b'/')),
            ),
            ("word", digits.union(lower).union(upper).union(ByteClass::just(b'_'))),
            ("one byte", ByteClass::just(b'q')),
            ("none", ByteClass::none()),
        ]
    }

    /// The tables answer membership for the class they were built from, on
    /// every byte there is. A table that is wrong about one byte refuses an
    /// input that holds it, which is a lost match rather than a slow one.
    #[test]
    fn the_nibble_tables_agree_with_the_class_on_every_byte() {
        for (name, class) in classes() {
            let tables = ClassTables::for_class(&class).expect("an ASCII class compiles");
            for b in 0..=255u8 {
                assert_eq!(tables.has(b), class.has(b), "{name} disagrees on byte {b:#04x}");
            }
        }
    }

    /// A class reaching past ASCII is refused rather than answered over its
    /// lower half.
    #[test]
    fn a_class_above_ascii_has_no_tables() {
        use crate::byte_nfa::ByteClass;
        assert!(ClassTables::for_class(&ByteClass::any()).is_none());
        assert!(ClassTables::for_class(&ByteClass::just(0x80)).is_none());
        assert!(ClassTables::for_class(&ByteClass::range(b'a', b'z')).is_some());
    }

    /// The vector path answers what the byte-at-a-time path answers. A faster
    /// reading that differs is not a reading.
    ///
    /// The inputs carry the cases the run arithmetic divides on: a run ending
    /// exactly at a thirty-two byte boundary, one spanning several, one
    /// opening the input, one closing it, and inputs shorter than one vector.
    #[test]
    fn the_vector_run_answers_what_the_scalar_run_answers() {
        let mut inputs: Vec<Vec<u8>> = vec![
            Vec::new(),
            b"q".to_vec(),
            b"abc def".to_vec(),
            b"a".repeat(31),
            b"a".repeat(32),
            b"a".repeat(33),
            b"a".repeat(1000),
            [b" ".repeat(32), b"a".repeat(32), b" ".repeat(32)].concat(),
            [b"a".repeat(30), b" ".to_vec(), b"a".repeat(40)].concat(),
            [b"a".repeat(64), b" ".to_vec()].concat(),
            [b" ".to_vec(), b"a".repeat(64)].concat(),
        ];
        // A run at every offset within a vector, so the head, tail and
        // interior arms are each reached with the carry set and clear.
        for k in 0..40usize {
            inputs.push([b" ".repeat(k), b"abcdef".to_vec(), b" ".repeat(40 - k)].concat());
        }
        for (name, class) in classes() {
            let tables = ClassTables::for_class(&class).expect("an ASCII class compiles");
            for input in &inputs {
                assert_eq!(
                    tables.longest_run(input),
                    tables.longest_run_scalar(input),
                    "{name} over {} bytes",
                    input.len()
                );
            }
        }
    }

    /// The bounded scan answers the threshold question the unbounded one
    /// answers, at every threshold around a run's own length. A scan that
    /// stops early and stops one byte too soon reads a present run as absent,
    /// which is a lost match.
    #[test]
    fn stopping_early_answers_the_threshold_the_full_scan_answers() {
        let inputs: Vec<Vec<u8>> = vec![
            Vec::new(),
            b"a".to_vec(),
            b"a".repeat(31),
            b"a".repeat(32),
            b"a".repeat(33),
            [b" ".repeat(40), b"a".repeat(20), b" ".repeat(40)].concat(),
            [b"a".repeat(10), b" ".to_vec(), b"a".repeat(20), b" ".to_vec(), b"a".repeat(5)]
                .concat(),
            [b"a".repeat(200), b" ".to_vec(), b"a".repeat(3)].concat(),
        ];
        for (name, class) in classes() {
            let tables = ClassTables::for_class(&class).expect("an ASCII class compiles");
            for input in &inputs {
                let longest = tables.longest_run(input);
                for least in 0..=(longest + 3) {
                    assert_eq!(
                        tables.has_run_of(input, least),
                        longest >= least,
                        "{name}, {} bytes, threshold {least} against a longest of {longest}",
                        input.len()
                    );
                }
            }
        }
    }

    /// The reading the refusal is built on: a longest run shorter than a
    /// kind's shortest token means the input holds no token of that kind.
    #[test]
    fn the_longest_run_is_what_a_necessary_condition_reads() {
        use crate::byte_nfa::ByteClass;
        let hex = ByteClass::range(b'0', b'9')
            .union(ByteClass::range(b'a', b'f'))
            .union(ByteClass::range(b'A', b'F'));
        let tables = ClassTables::for_class(&hex).expect("an ASCII class compiles");
        // A digest is thirty-two hex bytes at the least, so thirty-one refuses
        // and thirty-two does not.
        assert_eq!(tables.longest_run(&b"a".repeat(31)), 31);
        assert_eq!(tables.longest_run(&b"a".repeat(32)), 32);
        // Two short runs are not one long one.
        let split = [b"a".repeat(20), b" ".to_vec(), b"a".repeat(20)].concat();
        assert_eq!(tables.longest_run(&split), 20);
        // A byte outside the class ends a run at any position.
        assert_eq!(tables.longest_run(b"abcdefzabcdef"), 6);
    }
}
