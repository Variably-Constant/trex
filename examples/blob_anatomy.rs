//! Where do the prior's bytes go, and what could front-coding reach?
//!
//! Count quantization was measured and cannot help: twenty-one million rows
//! share a hundred thousand distinct count pairs, so the values are already
//! repeated two hundred times over. Whatever the blob costs, it is keys.
//!
//! This says how much, exactly, and what a shared-prefix encoding would leave.
//! Contexts are serialized sorted by their bytes, so consecutive keys within an
//! order already share prefixes; front-coding stores the shared length and the
//! suffix instead of the whole key. The saving is computable from the keys
//! themselves without encoding anything.
//!
//! What it cannot say is how much of that saving brotli already takes. These
//! are pre-compression byte counts, and the blob ships compressed, so the
//! front-coded figure here is an upper bound on what an explicit encoding could
//! add. Reported beside the real compressed size so the gap is visible rather
//! than assumed away.
//!
//! Run: `cargo run --release --example blob_anatomy -- <blob>`

use std::env;
use std::fs;

use trex::seam::{decode_baked, logistic_mix_bits};

fn main() {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: blob_anatomy <blob>");
        std::process::exit(2);
    };
    let blob = match fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            std::process::exit(1);
        }
    };
    let per_order = decode_baked(&blob);

    println!("{path}: {} bytes compressed", blob.len());
    println!(
        "{:>6} {:>12} {:>14} {:>14} {:>14} {:>10}",
        "order", "rows", "key bytes", "value bytes", "front-coded", "keys cut"
    );

    let mut tot_rows = 0usize;
    let mut tot_keys = 0usize;
    let mut tot_vals = 0usize;
    let mut tot_front = 0usize;
    for (oi, rows) in per_order.iter().enumerate() {
        if rows.is_empty() {
            continue;
        }
        let mut keys = 0usize;
        let mut vals = 0usize;
        // One byte of shared-prefix length, then the bytes that differ. The map
        // iterates in key order, which is the order the blob stores.
        let mut front = 0usize;
        let mut prev: &[u8] = &[];
        for (ctx, counts) in rows {
            keys += ctx.len();
            // Each pair is a symbol byte and a u16 count, plus one byte of pair
            // count for the row.
            vals += 1 + counts.len() * 3;
            let shared = ctx.iter().zip(prev).take_while(|(a, b)| a == b).count();
            front += 1 + (ctx.len() - shared);
            prev = ctx;
        }
        let cut = 100.0 * (1.0 - front as f64 / keys as f64);
        println!(
            "{:>6} {:>12} {keys:>14} {vals:>14} {front:>14} {cut:>9.1}%",
            crate_order(oi),
            rows.len()
        );
        tot_rows += rows.len();
        tot_keys += keys;
        tot_vals += vals;
        tot_front += front;
    }

    let raw = tot_keys + tot_vals;
    println!();
    println!("rows {tot_rows}");
    println!(
        "uncompressed: {raw} bytes, of which keys {tot_keys} ({:.1}%) and values {tot_vals} ({:.1}%)",
        100.0 * tot_keys as f64 / raw as f64,
        100.0 * tot_vals as f64 / raw as f64
    );
    println!(
        "front-coded keys would be {tot_front} bytes, {:.1}% of the keys, \
         taking the uncompressed total to {}",
        100.0 * tot_front as f64 / tot_keys as f64,
        tot_front + tot_vals
    );
    println!(
        "brotli already takes {raw} to {} ({:.2}x) for the whole blob",
        blob.len(),
        raw as f64 / blob.len() as f64
    );

    // Whether front-coding adds anything depends on how much of the prefix
    // sharing brotli already finds, and that is not arguable from the counts
    // above. Both key streams are built and compressed the same way, so the
    // difference between them is what an explicit encoding is worth on top.
    let mut plain: Vec<u8> = Vec::with_capacity(tot_keys);
    let mut coded: Vec<u8> = Vec::with_capacity(tot_front);
    for rows in &per_order {
        let mut prev: &[u8] = &[];
        for ctx in rows.keys() {
            plain.extend_from_slice(ctx);
            let shared = ctx.iter().zip(prev).take_while(|(a, b)| a == b).count();
            coded.push(shared as u8);
            coded.extend_from_slice(&ctx[shared..]);
            prev = ctx;
        }
    }
    let plain_br = brotli_len(&plain);
    let coded_br = brotli_len(&coded);
    println!();
    println!("{:>24} {:>14} {:>14} {:>10}", "key stream", "raw", "brotli q11", "vs plain");
    println!("{:>24} {:>14} {plain_br:>14} {:>9}", "as stored", plain.len(), "1.000x");
    println!(
        "{:>24} {:>14} {coded_br:>14} {:>8.3}x",
        "front-coded",
        coded.len(),
        coded_br as f64 / plain_br as f64
    );
    println!();
    println!("Below 1.000x an explicit prefix encoding beats what brotli finds on its");
    println!("own; at or above it, brotli was already taking that redundancy and the");
    println!("format change would buy nothing.");

    // Why brotli leaves sharing behind has two candidate explanations, and they
    // predict different things on a small slice. If the window is the reason -
    // 96 MB of keys against a 16 MB window at lgwin 24 - then a slice that fits
    // entirely inside the window should close the gap. If the reason is that a
    // back-reference costs more than one length byte for a short suffix, the gap
    // should hold at any size.
    let slice = 8_388_608usize.min(plain.len());
    let mut pslice: Vec<u8> = Vec::new();
    let mut cslice: Vec<u8> = Vec::new();
    {
        let mut taken = 0usize;
        let mut prev: &[u8] = &[];
        'outer: for rows in &per_order {
            for ctx in rows.keys() {
                if taken >= slice {
                    break 'outer;
                }
                pslice.extend_from_slice(ctx);
                let shared = ctx.iter().zip(prev).take_while(|(a, b)| a == b).count();
                cslice.push(shared as u8);
                cslice.extend_from_slice(&ctx[shared..]);
                prev = ctx;
                taken += ctx.len();
            }
        }
    }
    let ps = brotli_len(&pslice);
    let cs = brotli_len(&cslice);
    println!();
    println!(
        "the first {} bytes of keys, which fit inside brotli's window:",
        pslice.len()
    );
    println!("{:>24} {:>14} {:>10}", "key slice", "brotli q11", "vs plain");
    println!("{:>24} {ps:>14} {:>9}", "as stored", "1.000x");
    println!("{:>24} {cs:>14} {:>8.3}x", "front-coded", cs as f64 / ps as f64);
    println!();
    println!("A ratio here matching the full-stream one puts the gain in what a");
    println!("back-reference costs. A ratio nearer 1.000x puts it in the window, and");
    println!("would mean the gain is a property of this blob's size rather than its shape.");

    // Front-coding and a context-mixing coder both exploit shared prefixes, so
    // their gains may overlap rather than add. Coding both streams the same way
    // is what says which.
    //
    // Sampled by how many keys, not by how many bytes. The coded stream is
    // denser, so an equal number of bytes from each covers a different set of
    // keys and the ratio would compare two different contents. A sample at all
    // because the coder runs near 0.3 MB/s.
    let keys_sampled = 200_000usize;
    let mut psamp: Vec<u8> = Vec::new();
    let mut csamp: Vec<u8> = Vec::new();
    {
        let mut taken = 0usize;
        let mut prev: &[u8] = &[];
        'keys: for rows in &per_order {
            for ctx in rows.keys() {
                if taken >= keys_sampled {
                    break 'keys;
                }
                psamp.extend_from_slice(ctx);
                let shared = ctx.iter().zip(prev).take_while(|(a, b)| a == b).count();
                csamp.push(shared as u8);
                csamp.extend_from_slice(&ctx[shared..]);
                prev = ctx;
                taken += 1;
            }
        }
    }
    let plain_self = logistic_mix_bits(&psamp, true, &[], None) / 8.0;
    let coded_self = logistic_mix_bits(&csamp, true, &[], None) / 8.0;
    println!();
    println!(
        "the same {keys_sampled} keys under trex's own coder ({} bytes plain, {} coded):",
        psamp.len(),
        csamp.len()
    );
    println!("{:>24} {:>14} {:>10}", "key stream", "trex bytes", "vs plain");
    println!("{:>24} {:>14.0} {:>9}", "as stored", plain_self, "1.000x");
    println!(
        "{:>24} {:>14.0} {:>8.3}x",
        "front-coded",
        coded_self,
        coded_self / plain_self
    );
    println!();
    println!("If this ratio is near the brotli one, the two levers are taking the same");
    println!("redundancy and stack poorly. If it is nearer 1.000x, the coder was already");
    println!("finding the prefixes and front-coding adds nothing on top of it.");
}

/// Brotli-compressed length at the quality the blob ships at.
fn brotli_len(bytes: &[u8]) -> usize {
    let mut out = Vec::new();
    {
        let mut w = brotli::CompressorWriter::new(&mut out, 4096, 11, 24);
        std::io::Write::write_all(&mut w, bytes).expect("brotli encode");
    }
    out.len()
}

/// The order this position in [`trex::seam::MODEL_ORDERS`] stands for.
fn crate_order(oi: usize) -> usize {
    trex::seam::MODEL_ORDERS.get(oi).copied().unwrap_or(0)
}
