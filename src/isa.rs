//! Which instruction-set rung this CPU can run, resolved once.
//!
//! trex ships every rung of its SIMD ladders in one binary and lets the
//! running CPU choose, so the build stays portable and a machine with
//! AVX-512 uses it without a separate artifact. That choice has to be made
//! somewhere, and making it at each call site costs a load and a bit test
//! per call - the entropy pass would pay it once per chunk, the literal
//! search once per search.
//!
//! [`tier`] resolves the whole question once per process and hands back a
//! value every ladder can match on. `std::is_x86_feature_detected!` already
//! avoids re-issuing CPUID after its first use, so what this removes is the
//! repeated probe and the scattering of the policy, not the instruction.
//!
//! # The rung is a floor, not a menu
//!
//! [`Tier`] is ordered, and a rung implies every rung below it: a host that
//! reports [`Tier::Avx512`] can also run the AVX2 and SSE2 bodies. A ladder
//! matches from the top and falls through, so adding a rung above does not
//! disturb the ones under it.
//!
//! # What a rung promises
//!
//! Each names the exact features its bodies may use, because
//! `#[target_feature]` on a body the CPU cannot run is undefined behavior
//! and the promise here is what discharges it:
//!
//! - [`Tier::Avx512`] - `avx512f` and `avx512bw`, and everything below.
//! - [`Tier::Avx2`] - `avx2` and `bmi2`, and everything below.
//! - [`Tier::Sse2`] - `sse2`, which is architectural on x86-64.
//! - [`Tier::Scalar`] - nothing beyond the target's baseline. Every
//!   non-x86-64 architecture reports this, so the scalar body is the
//!   portable floor rather than a fallback that never runs.

use std::sync::OnceLock;

/// The highest ladder rung this CPU can run.
///
/// Ordered from least to most capable, so a ladder can compare rather than
/// enumerate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// The target's baseline instructions and nothing more.
    #[default]
    Scalar,
    /// `sse2`. Architectural on x86-64, so every x86-64 host reaches at
    /// least this.
    Sse2,
    /// `avx2` and `bmi2`.
    Avx2,
    /// `avx512f` and `avx512bw`.
    Avx512,
}

impl Tier {
    /// The rung's name, for a bench or a diagnostic that reports which body
    /// actually ran.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Tier::Scalar => "scalar",
            Tier::Sse2 => "sse2",
            Tier::Avx2 => "avx2",
            Tier::Avx512 => "avx512",
        }
    }
}

/// The rung this CPU can run, probed on first call and reused after.
///
/// A ladder calls this once and matches on the result, rather than probing
/// each feature at each call site.
#[must_use]
#[inline]
pub fn tier() -> Tier {
    static TIER: OnceLock<Tier> = OnceLock::new();
    *TIER.get_or_init(probe)
}

#[cfg(target_arch = "x86_64")]
fn probe() -> Tier {
    // Highest first: the rungs are cumulative, so the first match is the
    // answer and the tests below it would all pass too.
    if std::is_x86_feature_detected!("avx512f") && std::is_x86_feature_detected!("avx512bw") {
        return Tier::Avx512;
    }
    if std::is_x86_feature_detected!("avx2") && std::is_x86_feature_detected!("bmi2") {
        return Tier::Avx2;
    }
    if std::is_x86_feature_detected!("sse2") {
        return Tier::Sse2;
    }
    // Unreachable on a conforming x86-64, which guarantees sse2; kept
    // because the probe promises a total answer rather than assuming one.
    Tier::Scalar
}

#[cfg(not(target_arch = "x86_64"))]
fn probe() -> Tier {
    Tier::Scalar
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tier_is_stable_across_calls() {
        // A ladder reads this on a hot path and must not see it move.
        let first = tier();
        for _ in 0..1000 {
            assert_eq!(tier(), first, "the resolved tier must not change within a process");
        }
    }

    #[test]
    fn the_rungs_are_ordered_by_capability() {
        assert!(Tier::Avx512 > Tier::Avx2);
        assert!(Tier::Avx2 > Tier::Sse2);
        assert!(Tier::Sse2 > Tier::Scalar);
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn every_x86_64_host_reaches_at_least_sse2() {
        // sse2 is architectural on x86-64, so a Scalar answer there means
        // the probe failed rather than that the CPU is old.
        assert!(tier() >= Tier::Sse2, "x86-64 guarantees sse2; probe returned {:?}", tier());
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn the_tier_agrees_with_a_direct_probe() {
        // The cached answer must be the one the feature tests give, or the
        // ladder dispatches on something other than this CPU.
        let direct = if std::is_x86_feature_detected!("avx512f")
            && std::is_x86_feature_detected!("avx512bw")
        {
            Tier::Avx512
        } else if std::is_x86_feature_detected!("avx2") && std::is_x86_feature_detected!("bmi2") {
            Tier::Avx2
        } else {
            Tier::Sse2
        };
        assert_eq!(tier(), direct, "the cached tier disagrees with a direct feature probe");
        eprintln!("this host resolves to the {} rung", tier().name());
    }
}
