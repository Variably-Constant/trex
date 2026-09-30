//! Is the prior cheaper mapped than decoded, on the coder's real reads?
//!
//! `prior_mapped.rs` bounded the per-lookup cost of a mapped prior between two
//! synthetic access patterns: every key in table order (1.94x the heap table)
//! and a strided walk (18.99x). Real coding sits between them, and which end it
//! sits near decides whether mapping pays. This records the exact sequence of
//! prior reads the coder makes on a real text, then replays that sequence
//! against the decoded in-memory tables and against mapped open-addressing
//! tables laid out as `prior_mapped.rs` lays them out.
//!
//! Correctness first: both replays must sum the same counts. A control replays
//! the heap tables against themselves and must read 1.00x. Arm order rotates
//! every round, and each ratio is the median of per-round ratios.
//!
//! Run: `cargo run --release --example prior_access_replay -- <text> [out-dir]`

use std::env;
use std::fs::{self, File};
use std::hint::black_box;
use std::io::Write;
use std::time::Instant;

use memmap2::Mmap;
use trex::seam::{MODEL_ORDERS, load_byte_ngram, logistic_mix_bits, record_prior_keys, take_prior_keys};

const SLOT: usize = 16;
const ROUNDS: usize = 7;

fn capacity_for(n: usize) -> usize {
    ((n * 10) / 7).next_power_of_two().max(16)
}

fn build(rows: &[(u64, (u32, u32))], path: &str) -> std::io::Result<usize> {
    let cap = capacity_for(rows.len());
    let mut table = vec![0u8; cap * SLOT];
    let mask = cap - 1;
    for &(k, (a, b)) in rows {
        assert!(k != 0, "a zero key cannot be stored: an all-zero slot marks it empty");
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
            i = (i + 1) & mask;
        }
    }
    let mut f = File::create(path)?;
    f.write_all(&table)?;
    f.sync_all()?;
    Ok(cap)
}

fn get(map: &Mmap, cap: usize, k: u64) -> (u32, u32) {
    let mask = cap - 1;
    let mut i = (k as usize) & mask;
    loop {
        let off = i * SLOT;
        let here = u64::from_le_bytes(map[off..off + 8].try_into().expect("eight bytes"));
        if here == 0 {
            return (0, 0);
        }
        if here == k {
            let a = u32::from_le_bytes(map[off + 8..off + 12].try_into().expect("four bytes"));
            let b = u32::from_le_bytes(map[off + 12..off + 16].try_into().expect("four bytes"));
            return (a, b);
        }
        i = (i + 1) & mask;
    }
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn main() {
    let mut args = env::args().skip(1);
    let Some(text_path) = args.next() else {
        eprintln!("usage: prior_access_replay <text> [out-dir]");
        std::process::exit(2);
    };
    let dir = args.next().unwrap_or_else(|| "C:\\Temp\\prior_replay".to_string());
    if let Err(e) = fs::create_dir_all(&dir) {
        eprintln!("cannot create {dir}: {e}");
        std::process::exit(1);
    }
    let text = match fs::read(&text_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read {text_path}: {e}");
            std::process::exit(1);
        }
    };
    let blob = trex::seam::baked_model_blob();

    let t = Instant::now();
    let tables = load_byte_ngram(blob);
    let decode_ms = t.elapsed().as_secs_f64() * 1e3;
    let rows: usize = tables.iter().map(|(_, m)| m.len()).sum();
    println!("baked prior: {} bytes, {rows} rows, decode+expand {decode_ms:.1} ms", blob.len());

    record_prior_keys();
    let bits = logistic_mix_bits(&text, true, &[], Some(tables.as_slice()));
    let keys = take_prior_keys();
    println!(
        "{text_path}: {} bytes coded to {:.4} bits/byte, {} prior reads, {:.2} per byte",
        text.len(),
        bits / text.len() as f64,
        keys.len(),
        keys.len() as f64 / text.len() as f64
    );

    // Table position per recorded order, and the mapped form of each table.
    let position: Vec<usize> = (0..=255usize)
        .map(|o| tables.iter().position(|(k, _)| *k == o).unwrap_or(usize::MAX))
        .collect();
    for &(o, _) in &keys {
        assert!(position[o as usize] != usize::MAX, "order {o} was read but no table holds it");
    }
    assert!(MODEL_ORDERS.iter().all(|o| position[*o] != usize::MAX), "every model order has a table");

    let mut caps = Vec::with_capacity(tables.len());
    let mut paths = Vec::with_capacity(tables.len());
    let t = Instant::now();
    for (i, (_, m)) in tables.iter().enumerate() {
        let flat: Vec<(u64, (u32, u32))> = m.iter().map(|(&k, &v)| (k, v)).collect();
        let p = format!("{dir}\\order{i}.tbl");
        match build(&flat, &p) {
            Ok(cap) => {
                caps.push(cap);
                paths.push(p);
            }
            Err(e) => {
                eprintln!("cannot write {p}: {e}");
                std::process::exit(1);
            }
        }
    }
    let disk: usize = caps.iter().map(|c| c * SLOT).sum();
    println!("built mapped tables in {:.1} ms, {disk} bytes on disk", t.elapsed().as_secs_f64() * 1e3);
    let t = Instant::now();
    let maps: Vec<Mmap> = paths
        .iter()
        .map(|p| {
            let f = File::open(p).expect("the table just written");
            // SAFETY: written by this process and not modified while mapped;
            // the mapping is read-only and dropped before exit.
            unsafe { Mmap::map(&f).expect("map the table") }
        })
        .collect();
    println!("map: {:.3} ms against {decode_ms:.1} ms to decode", t.elapsed().as_secs_f64() * 1e3);

    let heap = || {
        let t = Instant::now();
        let mut sum = 0u64;
        for &(o, k) in &keys {
            let (a, b) = tables[position[o as usize]].1.get(&k).copied().unwrap_or((0, 0));
            sum = sum.wrapping_add(u64::from(a)).wrapping_add(u64::from(b) << 32);
        }
        (t.elapsed().as_secs_f64() * 1e9 / keys.len() as f64, black_box(sum))
    };
    let mapped = || {
        let t = Instant::now();
        let mut sum = 0u64;
        for &(o, k) in &keys {
            let p = position[o as usize];
            let (a, b) = get(&maps[p], caps[p], k);
            sum = sum.wrapping_add(u64::from(a)).wrapping_add(u64::from(b) << 32);
        }
        (t.elapsed().as_secs_f64() * 1e9 / keys.len() as f64, black_box(sum))
    };

    let (_, heap_sum) = heap();
    let (_, mapped_sum) = mapped();
    assert_eq!(mapped_sum, heap_sum, "the mapped replay summed different counts from the heap replay");
    println!("agreement: both replays sum the same counts over {} reads", keys.len());

    let arms: [&dyn Fn() -> (f64, u64); 3] = [&heap, &mapped, &heap];
    let (mut ns, mut control, mut ratio) = ([Vec::new(), Vec::new(), Vec::new()], Vec::new(), Vec::new());
    for round in 0..ROUNDS {
        let mut t = [0.0f64; 3];
        for i in 0..3 {
            let arm = (i + round) % 3;
            t[arm] = arms[arm]().0;
        }
        for (arm, v) in t.iter().enumerate() {
            ns[arm].push(*v);
        }
        control.push(t[0] / t[2]);
        ratio.push(t[1] / t[0]);
    }
    let (heap_ns, mapped_ns) = (median(&mut ns[0]), median(&mut ns[1]));
    let per_byte = keys.len() as f64 / text.len() as f64;
    let extra_ns_per_byte = (mapped_ns - heap_ns) * per_byte;
    println!(
        "replay: heap {heap_ns:.1} ns, mapped {mapped_ns:.1} ns a read, mapped/heap {:.2}x, control {:.3}x",
        median(&mut ratio),
        median(&mut control)
    );
    if extra_ns_per_byte > 0.0 {
        println!(
            "break-even: {decode_ms:.1} ms of decode saved / {extra_ns_per_byte:.1} ns more a byte = {:.1} MB of input",
            decode_ms * 1e6 / extra_ns_per_byte / 1e6
        );
    } else {
        println!("the mapped replay is no slower a read, so mapping saves the decode at every input size");
    }
}
