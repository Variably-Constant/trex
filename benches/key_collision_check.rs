//! Verify the u64 key index is collision-safe on the REAL shipped tables: per
//! level (universal / each language / each family), hash every distinct context
//! key and count how many collapse to the same u64. A collision would silently
//! map two different keys to one POS, so this must be ~zero before the u64 index
//! ships. Run: cargo run --release --example key_collision_check

use std::collections::{HashMap, HashSet};

fn fnv1a(b: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &x in b {
        h ^= u64::from(x);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn main() {
    for (path, label) in
        [("src/code_pos_table.tsv", "context"), ("src/code_pos_lexical.tsv", "lexical")]
    {
        let tsv = std::fs::read_to_string(path).expect("read table");
        let mut by_lvl: HashMap<&str, HashSet<&str>> = HashMap::new();
        for line in tsv.lines() {
            let mut f = line.split('\t');
            if let (Some(lvl), Some(key)) = (f.next(), f.next()) {
                by_lvl.entry(lvl).or_default().insert(key);
            }
        }
        let mut total_keys = 0usize;
        let mut total_coll = 0usize;
        let mut worst = ("", 0usize);
        for (lvl, keys) in &by_lvl {
            let hashes: HashSet<u64> = keys.iter().map(|k| fnv1a(k.as_bytes())).collect();
            let coll = keys.len() - hashes.len();
            total_keys += keys.len();
            total_coll += coll;
            if coll > worst.1 {
                worst = (lvl, coll);
            }
        }
        println!(
            "{label}: {total_keys} distinct keys across {} levels, {total_coll} collisions; worst level {:?} ({} collisions)",
            by_lvl.len(),
            worst.0,
            worst.1
        );
    }
}
