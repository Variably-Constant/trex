//! Does `SharedArray` cost anything over a hand-rolled read-only mapping?
//!
//! `examples/prior_mapped.rs` establishes what a mapped prior can reach: a flat
//! open-addressed table, sixteen bytes a slot, the key masked to an index. This
//! asks whether the primitive reaches the same figure, because a primitive that
//! is slower than the layout it wraps is not worth taking.
//!
//! What it adds is the one thing a hand-rolled mapping cannot have. The
//! prototype writes `SLOT = 16` in the builder and reads it in the prober; if
//! those ever disagreed the reader would address misaligned bytes, and every
//! probe would still return something that looks like a count. `SharedArray`
//! writes the stride into the header and compares it on every attach, so that
//! disagreement is refused rather than answered.
//!
//! The comparison is kept fair by giving both sides the same work: the same
//! table bytes, the same probe sequence, and lookups through `as_slice`, which
//! hands back the whole span so there is no per-element call. The
//! stride check happens once, at attach, and attach is timed separately.
//!
//! Run: `cargo run --release --example prior_shared_array -- <blob> [dir]`

use std::env;
use std::fs;
use std::time::Instant;

use subetha_cxc::shared_array::SharedArray;
use trex::seam::load_byte_ngram;

/// One stored row: the context key and its two counts.
const SLOT: u32 = 16;

/// Slots for `n` rows at a load factor near 0.7, rounded to a power of two so
/// the index is a mask rather than a division.
fn capacity_for(n: usize) -> u64 {
    (((n * 10) / 7).next_power_of_two().max(16)) as u64
}

/// Probe a flat table laid out as `cap` slots of [`SLOT`] bytes.
fn get(span: &[u8], cap: u64, k: u64) -> Option<(u32, u32)> {
    let mask = (cap - 1) as usize;
    let mut i = (k as usize) & mask;
    loop {
        let off = i * SLOT as usize;
        let here = u64::from_le_bytes(span[off..off + 8].try_into().expect("eight bytes"));
        if here == 0 {
            return None;
        }
        if here == k {
            let a = u32::from_le_bytes(span[off + 8..off + 12].try_into().expect("four bytes"));
            let b = u32::from_le_bytes(span[off + 12..off + 16].try_into().expect("four bytes"));
            return Some((a, b));
        }
        i = (i + 1) & mask;
    }
}

fn main() {
    let mut args = env::args().skip(1);
    let Some(blob_path) = args.next() else {
        eprintln!("usage: prior_shared_array <blob> [dir]");
        std::process::exit(2);
    };
    let dir = args.next().unwrap_or_else(|| "C:\\Temp\\prior_sa".to_string());
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

    let flat: Vec<Vec<(u64, (u32, u32))>> =
        tables.iter().map(|(_, m)| m.iter().map(|(&k, &v)| (k, v)).collect()).collect();

    let mut paths = Vec::new();
    let mut caps = Vec::new();
    let mut total = 0u64;
    let t = Instant::now();
    for (oi, rowset) in flat.iter().enumerate() {
        let cap = capacity_for(rowset.len());
        let p = format!("{dir}\\order{oi}.sa");
        let mut arr = match SharedArray::create(&p, cap, SLOT) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("cannot create {p} for {cap} slots: {e:?}");
                std::process::exit(1);
            }
        };
        let mask = cap - 1;
        let mut slot = [0u8; SLOT as usize];
        for &(k, (a, b)) in rowset {
            if k == 0 {
                continue;
            }
            let mut i = k & mask;
            loop {
                let here = match arr.get(i) {
                    Ok(s) => u64::from_le_bytes(s[0..8].try_into().expect("eight bytes")),
                    Err(e) => {
                        eprintln!("reading slot {i} of {cap} failed: {e:?}");
                        std::process::exit(1);
                    }
                };
                if here == k {
                    break;
                }
                if here == 0 {
                    slot[0..8].copy_from_slice(&k.to_le_bytes());
                    slot[8..12].copy_from_slice(&a.to_le_bytes());
                    slot[12..16].copy_from_slice(&b.to_le_bytes());
                    if let Err(e) = arr.set(i, &slot) {
                        eprintln!("writing slot {i} of {cap} failed: {e:?}");
                        std::process::exit(1);
                    }
                    break;
                }
                i = (i + 1) & mask;
            }
        }
        if let Err(e) = arr.seal() {
            eprintln!("sealing {p} failed: {e:?}");
            std::process::exit(1);
        }
        total += cap * u64::from(SLOT);
        caps.push(cap);
        paths.push(p);
    }
    let build_ms = t.elapsed().as_secs_f64() * 1e3;
    println!(
        "built and sealed {} arrays in {build_ms:.1} ms, {total} bytes ({} MB)",
        paths.len(),
        total / (1 << 20)
    );

    // Attach. The stride and length are checked here, once, against the header.
    let t = Instant::now();
    let arrays: Vec<SharedArray> = paths
        .iter()
        .zip(&caps)
        .map(|(p, &cap)| match SharedArray::open_read_only(p, cap, SLOT) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("attaching {p} failed: {e:?}");
                std::process::exit(1);
            }
        })
        .collect();
    let attach_ms = t.elapsed().as_secs_f64() * 1e3;
    println!("attach: {attach_ms:.3} ms  (against {decode_ms:.1} ms to decode)");

    // The guarantee, exercised rather than described: attaching with a stride
    // the header does not carry must be refused. A hand-rolled mapping has no
    // way to notice, and would answer every probe with misaligned bytes.
    match SharedArray::open_read_only(&paths[0], caps[0], SLOT + 4) {
        Ok(_) => {
            eprintln!("a wrong stride was accepted; the header check is not doing its job");
            std::process::exit(1);
        }
        Err(e) => println!("stride check: a wrong stride is refused at attach, as {e:?}"),
    }

    // Correctness before speed: every row must come back with the value the
    // decoded table holds.
    let spans: Vec<&[u8]> = arrays.iter().map(SharedArray::as_slice).collect();
    let mut checked = 0usize;
    for (oi, rowset) in flat.iter().enumerate() {
        for &(k, v) in rowset.iter().take(200_000) {
            if k == 0 {
                continue;
            }
            assert_eq!(get(spans[oi], caps[oi], k), Some(v), "order {oi} key {k:#x} disagrees");
            checked += 1;
        }
    }
    println!("agreement: {checked} probes match the decoded table exactly");

    for (label, stride) in [("sequential", 1usize), ("strided", 7919)] {
        let mut acc = 0u64;
        let t = Instant::now();
        let mut probes = 0usize;
        for (oi, rowset) in flat.iter().enumerate() {
            let n = rowset.len();
            let mut i = 0usize;
            while probes < 2_000_000 && i < n {
                let (k, _) = rowset[i];
                if k != 0 && let Some((a, _)) = get(spans[oi], caps[oi], k) {
                    acc = acc.wrapping_add(u64::from(a));
                }
                probes += 1;
                i += stride;
            }
        }
        let sa_ns = t.elapsed().as_secs_f64() * 1e3 * 1e6 / probes as f64;

        let mut acc2 = 0u64;
        let t = Instant::now();
        let mut p2 = 0usize;
        for (oi, rowset) in flat.iter().enumerate() {
            let n = rowset.len();
            let mut i = 0usize;
            while p2 < 2_000_000 && i < n {
                let (k, _) = rowset[i];
                if let Some(&(a, _)) = tables[oi].1.get(&k) {
                    acc2 = acc2.wrapping_add(u64::from(a));
                }
                p2 += 1;
                i += stride;
            }
        }
        let heap_ns = t.elapsed().as_secs_f64() * 1e3 * 1e6 / p2 as f64;
        assert_eq!(acc, acc2, "{label}: the two paths summed different values");
        println!(
            "{label:>11}: {probes} probes  SharedArray {sa_ns:>6.1} ns  heap {heap_ns:>6.1} ns  {:>5.2}x",
            sa_ns / heap_ns
        );
    }

    println!();
    println!("Compare the per-probe figures against examples/prior_mapped.rs on the");
    println!("same blob: the layout is identical, so a difference is what the");
    println!("primitive costs over addressing the mapping directly.");
}
