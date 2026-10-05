//! The keyed hash the single-pass engine's register-dedup set uses.
//!
//! One rotate, one xor and one multiply a word, against SipHash-1-3's round a
//! word, on a set the scan inserts into once per thread per step. The state
//! starts from a key drawn once per process, so the bucket a register state
//! lands in differs between runs of the same input.
//!
//! This is not a cryptographic hash. The key raises the cost of finding
//! colliding register states from reading a fixed table to probing the
//! running process, and an attacker who learns the key can construct
//! collisions.

use std::hash::{BuildHasher, Hasher, RandomState};
use std::sync::OnceLock;

/// FxHash's mixing constant: the odd multiplier its chain carries.
const FX_MULT: u64 = 0x517c_c1b7_2722_0a95;

/// The offset the keyless chain starts from, FNV's 64-bit basis.
const FX_BASIS: u64 = 0xcbf2_9ce4_8422_2325;

/// This process's key, drawn once from the standard library's random state.
fn process_key() -> u64 {
    static KEY: OnceLock<u64> = OnceLock::new();
    *KEY.get_or_init(|| RandomState::new().hash_one(FX_MULT))
}

/// A hash state that mixes one word a step.
#[derive(Clone, Copy)]
pub(crate) struct FxHasher(u64);

impl FxHasher {
    #[inline]
    fn mix(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(FX_MULT);
    }
}

impl Hasher for FxHasher {
    /// Whole words first, then whatever tail is left, a byte a step. The
    /// signed and 128-bit writes reach this through the trait's own
    /// forwarding.
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let (words, tail) = bytes.as_chunks::<8>();
        for w in words {
            self.mix(u64::from_le_bytes(*w));
        }
        for &b in tail {
            self.mix(u64::from(b));
        }
    }

    #[inline]
    fn write_u8(&mut self, n: u8) {
        self.mix(u64::from(n));
    }

    #[inline]
    fn write_u16(&mut self, n: u16) {
        self.mix(u64::from(n));
    }

    #[inline]
    fn write_u32(&mut self, n: u32) {
        self.mix(u64::from(n));
    }

    #[inline]
    fn write_u64(&mut self, n: u64) {
        self.mix(n);
    }

    #[inline]
    fn write_usize(&mut self, n: usize) {
        self.mix(n as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
}

/// The avalanche a hash needs before a map reads a bucket from one end of it
/// and a control byte from the other.
///
/// [`FxHasher`] mixes a word a step and stops, and its last act is a multiply,
/// which carries influence one way along the word and leaves the other end
/// poorly distributed. `echo`'s repeat table already carries the crate's hash
/// through these five steps before taking an index from it, for that reason;
/// this is the same steps, named once so a map can take them too.
#[inline]
pub(crate) fn avalanche(mut h: u64) -> u64 {
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    h ^ (h >> 33)
}

/// [`FxHasher`] with [`avalanche`] over what it finishes with.
///
/// For a key a map hashes, rather than a key a table indexes: hashbrown takes
/// its bucket from one end of the hash and its control byte from the other, so
/// both ends decide where a key lands. Plain [`FxBuild`] measured 19% worse
/// than SipHash on byte-string keys differing only in a trailing serial number,
/// which is the shape an identifier in real source has.
#[derive(Clone, Copy)]
pub(crate) struct FxFinalHasher(FxHasher);

impl Hasher for FxFinalHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        self.0.write(bytes);
    }

    #[inline]
    fn write_u8(&mut self, n: u8) {
        self.0.write_u8(n);
    }

    #[inline]
    fn write_u16(&mut self, n: u16) {
        self.0.write_u16(n);
    }

    #[inline]
    fn write_u32(&mut self, n: u32) {
        self.0.write_u32(n);
    }

    #[inline]
    fn write_u64(&mut self, n: u64) {
        self.0.write_u64(n);
    }

    #[inline]
    fn write_usize(&mut self, n: usize) {
        self.0.write_usize(n);
    }

    #[inline]
    fn finish(&self) -> u64 {
        avalanche(self.0.finish())
    }
}

/// Builds [`FxFinalHasher`]s from this process's key.
#[derive(Clone, Copy, Default)]
pub(crate) struct FxFinalBuild(FxBuild);

impl BuildHasher for FxFinalBuild {
    type Hasher = FxFinalHasher;

    #[inline]
    fn build_hasher(&self) -> FxFinalHasher {
        FxFinalHasher(self.0.build_hasher())
    }
}

/// Builds [`FxHasher`]s from this process's key.
#[derive(Clone, Copy)]
pub(crate) struct FxBuild(u64);

impl FxBuild {
    /// A builder carrying the key this process drew.
    pub(crate) fn process() -> Self {
        FxBuild(process_key())
    }
}

impl Default for FxBuild {
    fn default() -> Self {
        Self::process()
    }
}

impl BuildHasher for FxBuild {
    type Hasher = FxHasher;

    #[inline]
    fn build_hasher(&self) -> FxHasher {
        FxHasher(self.0 ^ FX_BASIS)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::hash::BuildHasher;

    use super::{FxBuild, FxFinalBuild, process_key};

    /// The finalizer must never merge two hashes.
    ///
    /// Injective is the exact property, so it needs no threshold to state: a
    /// mixing step that mapped two distinct hashes together would make two keys
    /// collide that the hash itself had separated, and no amount of good
    /// distribution elsewhere repairs that.
    ///
    /// Zero is a fixed point, because every step of a xor-shift-and-multiply
    /// chain maps it to itself. A value mapped to itself is not two values
    /// brought together, so it costs a key nothing, and the chain is injective
    /// with it.
    ///
    /// The distribution claim is deliberately left unasserted. Counting how many
    /// buckets each form fills over 256 keys reads 194 against 198 - a
    /// difference birthday collisions account for entirely, at a sparseness the
    /// real map, which holds 163,844 entries, never sees. What justifies the
    /// finalized form is the measured cost of the pass that uses it, not a count
    /// taken at the wrong scale.
    #[test]
    fn the_finalizer_merges_no_two_hashes() {
        let mut seen = HashSet::with_capacity(1 << 16);
        for i in 0..1u64 << 16 {
            assert!(
                seen.insert(super::avalanche(i)),
                "the finalizer merged {i} onto another hash"
            );
        }
        assert_eq!(super::avalanche(0), 0, "zero is the chain's fixed point");
        // And over the shape the echo keys have, where what differs between one
        // key and the next is the low bytes.
        let mut seen = HashSet::with_capacity(1 << 16);
        for i in 0..1u64 << 16 {
            assert!(
                seen.insert(super::avalanche(0x1234_5678_9abc_0000 | i)),
                "the finalizer merged two keys differing in a trailing serial"
            );
        }
    }

    /// The avalanche must open out the end hashbrown picks a bucket by, read at
    /// the scale the echo key map works at.
    ///
    /// That map holds 163,844 entries, counted by `echo: map probes that write
    /// an entry`, and its keys are identifiers differing in a trailing serial.
    /// hashbrown chooses a bucket by the low bits of a hash and a control byte
    /// by the high seven, and it is the bucket that decides how far a probe
    /// walks.
    ///
    /// Distinct values each form takes at the bucket end over those keys:
    ///
    ///   SipHash            121,790
    ///   fxhash, finalized  106,660    88% of it
    ///   fxhash, raw         61,308    50% of it
    ///
    /// So the raw form fills half the buckets the hasher it replaced does, which
    /// is the low end and is the arithmetic of the chain: `mix` ends in a
    /// multiply, a multiply carries influence upward, and the low bits of a
    /// product depend only on the low bits of its operands. The avalanche's
    /// xor-shifts carry the high bits back down and recover most of the gap.
    ///
    /// It does not close the gap, and it does not need to: the finalized form
    /// is cheaper to compute than SipHash, and 88% of the buckets at a fraction
    /// of the cost is what measured 28% off the pass that uses it. The
    /// assertion is the ordering rather than a fraction, because the fraction
    /// is a reading and the ordering is the property.
    #[test]
    fn the_avalanche_opens_out_the_bucket_end() {
        const KEYS: usize = 163_844;
        let keys: Vec<String> = (100_000..100_000 + KEYS).map(|i| format!("value_{i}")).collect();
        // The low eighteen bits index a table sized for this many keys.
        let buckets = |hs: &[u64]| {
            let mut v: Vec<u64> = hs.iter().map(|h| h & 0x3_ffff).collect();
            v.sort_unstable();
            v.dedup();
            v.len()
        };
        let raw = buckets(&keys.iter().map(|k| FxBuild::process().hash_one(k)).collect::<Vec<_>>());
        let mixed =
            buckets(&keys.iter().map(|k| FxFinalBuild::default().hash_one(k)).collect::<Vec<_>>());
        assert!(
            mixed > raw,
            "the avalanche left the bucket end where it was: {mixed} against {raw}"
        );
    }

    #[test]
    fn one_key_hashes_equal_values_equally() {
        let b = FxBuild::process();
        assert_eq!(b.hash_one((7usize, [1usize, 2, 3])), b.hash_one((7usize, [1usize, 2, 3])));
        assert_eq!(process_key(), process_key());
        assert_eq!(FxBuild::process().hash_one(5u64), b.hash_one(5u64));
    }

    #[test]
    fn another_key_moves_the_hash() {
        assert_ne!(FxBuild(1).hash_one(9u64), FxBuild(2).hash_one(9u64));
    }

    #[test]
    fn neighboring_register_keys_land_apart() {
        let b = FxBuild::process();
        let mut seen = HashSet::new();
        for pc in 0..64usize {
            for slot in 0..64usize {
                assert!(seen.insert(b.hash_one((pc, slot))), "pc {pc} with slot {slot} collided");
            }
        }
    }

    #[test]
    fn a_set_keyed_with_it_dedups_as_the_standard_one_does() {
        let mut ours: HashSet<(usize, usize), FxBuild> = HashSet::with_hasher(FxBuild::process());
        let mut theirs: HashSet<(usize, usize)> = HashSet::new();
        for pc in 0..200usize {
            for slot in 0..3usize {
                let key = (pc % 50, slot);
                assert_eq!(ours.insert(key), theirs.insert(key), "at {key:?}");
            }
        }
        assert_eq!(ours.len(), theirs.len());
    }
}
