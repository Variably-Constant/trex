//! Measure the context-key index: the tagger looks up a context key (a 20-30 char
//! string built per token) in a per-language / universal table millions of times
//! across a corpus. This benches three index representations on the REAL shipped
//! keys so we keep only what is actually faster on this hardware:
//!
//!   1. `HashMap<String>` with the default SipHash (what the tagger ships today)
//!   2. `HashMap<String>` with a fast FNV-1a hasher (isolates the hasher cost)
//!   3. `HashMap<u64>` with an identity hasher over a precomputed FNV-1a key hash
//!      (the "hash the big string into a small fixed key for indexing" idea)
//!
//! Run: cargo run --release --example key_index_bench

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::time::Instant;

/// FNV-1a 64-bit, the fast non-cryptographic hash of the key bytes.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// A `Hasher` that streams bytes through FNV-1a (for `HashMap<String, _>`).
#[derive(Default)]
struct Fnv(u64);
impl Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        let mut h = if self.0 == 0 { 0xcbf2_9ce4_8422_2325u64 } else { self.0 };
        for &b in bytes {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        self.0 = h;
    }
}

/// An identity `Hasher` for `u64` keys - the key already IS a good hash.
#[derive(Default)]
struct IdHash(u64);
impl Hasher for IdHash {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, _: &[u8]) {
        unreachable!("identity hasher only takes write_u64")
    }
    fn write_u64(&mut self, n: u64) {
        self.0 = n;
    }
}

fn main() {
    let tsv = std::fs::read_to_string("src/code_pos_table.tsv").expect("read table");
    let mut keys: Vec<String> = tsv
        .lines()
        .filter_map(|l| l.split('\t').nth(1))
        .map(str::to_string)
        .collect();
    keys.sort();
    keys.dedup();
    keys.truncate(30_000);
    let avg: f64 = keys.iter().map(String::len).sum::<usize>() as f64 / keys.len() as f64;
    println!("{} distinct keys, avg {avg:.1} chars", keys.len());

    let mut s_sip: HashMap<String, u8> = HashMap::new();
    let mut s_fnv: HashMap<String, u8, BuildHasherDefault<Fnv>> = HashMap::default();
    let mut u_id: HashMap<u64, u8, BuildHasherDefault<IdHash>> = HashMap::default();
    for (i, k) in keys.iter().enumerate() {
        let v = (i % 13) as u8;
        s_sip.insert(k.clone(), v);
        s_fnv.insert(k.clone(), v);
        u_id.insert(fnv1a(k.as_bytes()), v);
    }

    let n = 8_000_000usize;
    let probes: Vec<&str> = keys.iter().map(String::as_str).cycle().take(n).collect();

    // Several readings an arm, reduced by the middle one. The rows below are
    // divided into each other and reported as speedups, and a ratio of two
    // single samples carries whatever either of them happened to catch.
    const ROUNDS: usize = 5;
    let bench = |label: &str, f: &dyn Fn() -> u64| -> f64 {
        core::hint::black_box(f()); // warm caches
        let mut taken = Vec::with_capacity(ROUNDS);
        let mut acc = 0;
        for _ in 0..ROUNDS {
            let t = Instant::now();
            acc = f();
            #[allow(clippy::cast_precision_loss)]
            let ns = t.elapsed().as_nanos() as f64 / n as f64;
            taken.push(ns);
        }
        taken.sort_by(f64::total_cmp);
        let ns = taken[taken.len() / 2];
        println!(
            "  {label:<22} {ns:6.2} ns/lookup   (acc {acc}, {:.2} to {:.2})",
            taken[0],
            taken[taken.len() - 1]
        );
        ns
    };

    println!("=== {n} lookups (all hits) ===");
    let sip = bench("HashMap<String> sip", &|| {
        let mut a = 0u64;
        for k in &probes {
            if let Some(v) = s_sip.get(*k) {
                a += u64::from(*v);
            }
        }
        a
    });
    let fnv = bench("HashMap<String> fnv", &|| {
        let mut a = 0u64;
        for k in &probes {
            if let Some(v) = s_fnv.get(*k) {
                a += u64::from(*v);
            }
        }
        a
    });
    let u64m = bench("HashMap<u64> identity", &|| {
        let mut a = 0u64;
        for k in &probes {
            if let Some(v) = u_id.get(&fnv1a(k.as_bytes())) {
                a += u64::from(*v);
            }
        }
        a
    });
    println!("\nspeedup vs shipped (SipHash String):");
    println!("  HashMap<String> fnv : {:.2}x", sip / fnv);
    println!("  HashMap<u64> identity: {:.2}x", sip / u64m);
}
