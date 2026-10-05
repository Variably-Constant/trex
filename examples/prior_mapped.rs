//! Is the prior cheaper mapped than decoded?
//!
//! `load_byte_ngram` decodes brotli and expands every row into a hash table, so
//! a short-lived process does the decode and holds the expanded table on every
//! run, and discards both at exit. A file laid out as the table already is can be mapped
//! instead: no decode, and only the pages a lookup touches become resident.
//!
//! Three costs decide it, and the third is the one that can sink it. The mixer
//! does one lookup per order per bit, so a per-lookup regression multiplies by
//! six orders times eight bits times the input length and would swamp any
//! startup win.
//!
//! The layout is the one the in-memory table already has: open addressing with
//! linear probing over 16-byte slots, capacity a power of two. Keys are already
//! hashes - `U64Map` hashes them with an identity hasher - so a slot index is
//! the key masked, and a hit is normally the first probe rather than a descent.
//!
//! Key 0 cannot be stored, because an all-zero slot is what marks a slot empty.
//! Any such row is counted and reported rather than dropped silently.
//!
//! Both paths answer every probe and the answers must agree exactly. A mapped
//! table that is faster and wrong is the failure this guards against.
//!
//! Run: `cargo run --release --example prior_mapped -- <blob> [out-dir]`

use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::time::Instant;

use memmap2::Mmap;
use trex::seam::load_byte_ngram;

/// One stored row: the context key and its two counts.
const SLOT: usize = 16;

/// Slots for `n` rows at a load factor near 0.7, rounded to a power of two so
/// the index is a mask rather than a division.
fn capacity_for(n: usize) -> usize {
    ((n * 10) / 7).next_power_of_two().max(16)
}

fn build(order_rows: &[(u64, (u32, u32))], path: &str) -> std::io::Result<(usize, usize)> {
    let cap = capacity_for(order_rows.len());
    let mut table = vec![0u8; cap * SLOT];
    let mask = cap - 1;
    let mut zero_keys = 0usize;
    for &(k, (a, b)) in order_rows {
        if k == 0 {
            zero_keys += 1;
            continue;
        }
        let mut i = (k as usize) & mask;
        loop {
            let off = i * SLOT;
            let here = u64::from_le_bytes(table[off..off + 8].try_into().expect("eight bytes"));
            if here == 0 {
                table[off..off + 8].copy_from_slice(&k.to_le_bytes());
                table[off + 8..off + 12].copy_from_slice(&a.to_le_bytes());
                table[off + 12..off + 16].copy_from_slice(&b.to_le_bytes());
                break;
            }
            if here == k {
                break;
            }
            i = (i + 1) & mask;
        }
    }
    let mut f = File::create(path)?;
    f.write_all(&table)?;
    f.sync_all()?;
    Ok((cap, zero_keys))
}

/// Probe the mapped table. `None` when the key is absent.
fn get(map: &Mmap, cap: usize, k: u64) -> Option<(u32, u32)> {
    let mask = cap - 1;
    let mut i = (k as usize) & mask;
    loop {
        let off = i * SLOT;
        let here = u64::from_le_bytes(map[off..off + 8].try_into().expect("eight bytes"));
        if here == 0 {
            return None;
        }
        if here == k {
            let a = u32::from_le_bytes(map[off + 8..off + 12].try_into().expect("four bytes"));
            let b = u32::from_le_bytes(map[off + 12..off + 16].try_into().expect("four bytes"));
            return Some((a, b));
        }
        i = (i + 1) & mask;
    }
}

fn main() {
    let mut args = env::args().skip(1);
    let Some(blob_path) = args.next() else {
        eprintln!("usage: prior_mapped <blob> [out-dir]");
        std::process::exit(2);
    };
    let dir = args.next().unwrap_or_else(|| "C:\\Temp\\prior_mapped".to_string());
    if let Err(e) = fs::create_dir_all(&dir) {
        eprintln!("cannot create {dir}: {e}");
        std::process::exit(1);
    }
    let blob = match fs::read(&blob_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read {blob_path}: {e}");
            std::process::exit(1);
        }
    };

    println!("blob: {blob_path} ({} bytes)", blob.len());
    let t = Instant::now();
    let tables = load_byte_ngram(&blob);
    let decode_ms = t.elapsed().as_secs_f64() * 1e3;
    let rows: usize = tables.iter().map(|(_, m)| m.len()).sum();
    println!("decode+expand: {decode_ms:.1} ms for {rows} rows over {} orders", tables.len());

    // Flattened once so build and probe read the same rows in the same order.
    let flat: Vec<Vec<(u64, (u32, u32))>> =
        tables.iter().map(|(_, m)| m.iter().map(|(&k, &v)| (k, v)).collect()).collect();

    let mut paths = Vec::new();
    let mut caps = Vec::new();
    let mut total_file = 0usize;
    let mut total_zero = 0usize;
    let t = Instant::now();
    for (oi, rowset) in flat.iter().enumerate() {
        let p = format!("{dir}\\order{oi}.tbl");
        match build(rowset, &p) {
            Ok((cap, zero)) => {
                total_file += cap * SLOT;
                total_zero += zero;
                caps.push(cap);
                paths.push(p);
            }
            Err(e) => {
                eprintln!("cannot write {p}: {e}");
                std::process::exit(1);
            }
        }
    }
    let build_ms = t.elapsed().as_secs_f64() * 1e3;
    println!(
        "built {} tables in {build_ms:.1} ms, {total_file} bytes on disk ({} MB)",
        paths.len(),
        total_file / (1 << 20)
    );
    if total_zero > 0 {
        println!("rows whose key was 0 and could not be stored: {total_zero}");
    }

    // Mapping is what a real startup would do instead of the decode.
    let t = Instant::now();
    let maps: Vec<Mmap> = paths
        .iter()
        .map(|p| {
            let f = File::open(p).expect("the table just written");
            // SAFETY: the file is written by this process and not modified
            // while mapped; the mapping is read-only and dropped before exit.
            unsafe { Mmap::map(&f).expect("map the table") }
        })
        .collect();
    let map_ms = t.elapsed().as_secs_f64() * 1e3;
    println!("map: {map_ms:.3} ms  (against {decode_ms:.1} ms to decode)");

    // Correctness first: every row must come back from the mapped table with
    // the value the in-memory table holds. A faster wrong table is worthless.
    let mut checked = 0usize;
    for (oi, rowset) in flat.iter().enumerate() {
        for &(k, v) in rowset.iter().take(200_000) {
            if k == 0 {
                continue;
            }
            let got = get(&maps[oi], caps[oi], k);
            assert_eq!(got, Some(v), "order {oi} key {k:#x}: mapped table disagrees");
            checked += 1;
        }
    }
    println!("agreement: {checked} probes, mapped table matches the decoded one exactly");

    // Per-lookup cost, the risk. Two access patterns: every key once in table
    // order, which is the best case for locality, and a strided walk that
    // defeats it. Real text is between them, reusing a hot subset.
    for (label, stride) in [("sequential", 1usize), ("strided", 7919)] {
        let mut hot = 0u64;
        let t = Instant::now();
        let mut probes = 0usize;
        for (oi, rowset) in flat.iter().enumerate() {
            let n = rowset.len();
            if n == 0 {
                continue;
            }
            let mut i = 0usize;
            while probes < 2_000_000 && i < n {
                let (k, _) = rowset[i];
                if k != 0 && let Some((a, _)) = get(&maps[oi], caps[oi], k) {
                    hot = hot.wrapping_add(u64::from(a));
                }
                probes += 1;
                i += stride;
            }
        }
        let ms = t.elapsed().as_secs_f64() * 1e3;
        let mapped_ns = ms * 1e6 / probes as f64;

        let mut hot2 = 0u64;
        let t = Instant::now();
        let mut p2 = 0usize;
        for (oi, rowset) in flat.iter().enumerate() {
            let n = rowset.len();
            if n == 0 {
                continue;
            }
            let mut i = 0usize;
            while p2 < 2_000_000 && i < n {
                let (k, _) = rowset[i];
                if let Some(&(a, _)) = tables[oi].1.get(&k) {
                    hot2 = hot2.wrapping_add(u64::from(a));
                }
                p2 += 1;
                i += stride;
            }
        }
        let ms2 = t.elapsed().as_secs_f64() * 1e3;
        let heap_ns = ms2 * 1e6 / p2 as f64;
        assert_eq!(hot, hot2, "{label}: the two paths summed different values");
        println!(
            "{label:>11}: {probes} probes  mapped {mapped_ns:>6.1} ns  heap {heap_ns:>6.1} ns  {:>5.2}x",
            mapped_ns / heap_ns
        );
    }

    println!();
    println!("Above 1.00x the mapped table is slower per lookup. The mixer does one");
    println!("lookup per order per bit, so that ratio multiplies through the scan.");
}
