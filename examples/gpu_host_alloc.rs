//! Is the GPU host path superlinear because of its work or its allocations?
//!
//! `significant` and `select` in `src/gpu.rs` are both a single pass over the
//! tokens with their output vectors reserved up front, so both are O(n) by
//! inspection. Across the phase sweep they grow 888x and 1350x for 512x the
//! tokens.
//!
//! The two differ in what they allocate. At four million tokens `significant`
//! reserves about fifty megabytes and `select` about seventeen, each freed
//! when the call returns. An allocation that size is served by the operating
//! system rather than out of the allocator's arena and is returned on free, so
//! every call faults every page in again. That cost is proportional to bytes
//! allocated, not to work done, and it is absent at small n.
//!
//! This times each pass twice at the same n: once allocating as production
//! does, and once filling buffers allocated before the clock starts. Work is
//! identical in both; only the allocation moves. If the reused column is
//! linear and the fresh column is not, the growth is allocation.
//!
//! Run: `cargo run --release --example gpu_host_alloc`

use std::time::Instant;

use trex::Span;
use trex::token::{Token, TokenKind};

/// The corpus `benches/gpu_throughput.rs --phases` builds, so the token
/// stream here is the one the phase sweep measured rather than a model of it.
fn corpus(pairs: usize) -> String {
    let mut input = String::new();
    for i in 0..pairs {
        input.push_str("tag ");
        input.push_str(&(i % 1000).to_string());
        input.push(' ');
    }
    input
}

fn fill_significant(toks: &[Token], kinds: &mut Vec<u32>, spans: &mut Vec<(u32, u32)>) {
    for t in toks {
        if t.kind != TokenKind::Whitespace {
            kinds.push(t.kind.code());
            spans.push((t.start, t.end));
        }
    }
}

fn fill_select(spans: &[(u32, u32)], result: &[i32], out: &mut Vec<Span>) {
    let n = spans.len();
    let mut a = 0usize;
    while a < n {
        let best = result[a];
        if best > a as i32 {
            let e = best as usize;
            out.push(Span { start: spans[a].0, end: spans[e - 1].1 });
            a = e;
        } else {
            a += 1;
        }
    }
}

/// The middle sample, as the phase bench reports.
fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn main() {
    println!("{:>10}  {:>12} {:>12} {:>8}   {:>12} {:>12} {:>8}", "tokens", "prep fresh", "prep reused", "ratio", "sel fresh", "sel reused", "ratio");

    // The pair counts the phase sweep uses, so the rows line up with it.
    let sizes = [4_096usize, 16_384, 65_536, 262_144, 1_048_576, 2_097_152];
    let mut base: Option<(f64, f64, f64, f64)> = None;

    for pairs in sizes {
        let input = corpus(pairs);
        let toks = trex::parallel_lex::lex_parallel(input.as_bytes());
        let sig = toks.iter().filter(|t| t.kind != TokenKind::Whitespace).count();
        // `\W \N` matches two tokens from each anchor, as the sweep's pattern does.
        let result: Vec<i32> = (0..sig).map(|i| (i + 2) as i32).collect();
        let spans: Vec<(u32, u32)> = toks
            .iter()
            .filter(|t| t.kind != TokenKind::Whitespace)
            .map(|t| (t.start, t.end))
            .collect();
        let n = sig;

        // Median of the reps, as `benches/gpu_throughput.rs --phases` reports,
        // so a row here can be read against a row there. A minimum would be the
        // best case and would not compare.
        let mut prep_fresh_s: Vec<f64> = Vec::new();
        let mut prep_reused_s: Vec<f64> = Vec::new();
        let mut sel_fresh_s: Vec<f64> = Vec::new();
        let mut sel_reused_s: Vec<f64> = Vec::new();

        // Buffers allocated once, before any clock starts, and cleared between
        // rounds. `clear` keeps the capacity, so the pages stay faulted in.
        // Prep as the scan meets it: straight off a full parallel lex, whose
        // workers have just written the token stream across every core. The
        // rounds below re-run it on a stream already read once, so the pair
        // separates the cost of the pass from the cost of reading a stream
        // just evicted.
        let cold_input = corpus(pairs);
        let cold_toks = trex::parallel_lex::lex_parallel(cold_input.as_bytes());
        let mut k_cold: Vec<u32> = Vec::with_capacity(sig);
        let mut s_cold: Vec<(u32, u32)> = Vec::with_capacity(sig);
        let t = Instant::now();
        fill_significant(&cold_toks, &mut k_cold, &mut s_cold);
        let prep_after_lex = t.elapsed().as_secs_f64() * 1e6;
        assert!(!k_cold.is_empty());
        drop((cold_toks, k_cold, s_cold, cold_input));

        let mut k_warm: Vec<u32> = Vec::with_capacity(sig);
        let mut s_warm: Vec<(u32, u32)> = Vec::with_capacity(sig);
        let mut o_warm: Vec<Span> = Vec::with_capacity(sig / 2 + 1);
        fill_significant(&toks, &mut k_warm, &mut s_warm);
        fill_select(&spans, &result, &mut o_warm);

        for _ in 0..5 {
            let t = Instant::now();
            let mut kinds: Vec<u32> = Vec::with_capacity(sig);
            let mut sp: Vec<(u32, u32)> = Vec::with_capacity(sig);
            fill_significant(&toks, &mut kinds, &mut sp);
            let e = t.elapsed().as_secs_f64() * 1e6;
            // Reading one element keeps the fill from being optimized away.
            assert!(!kinds.is_empty() && !sp.is_empty());
            prep_fresh_s.push(e);
            drop((kinds, sp));

            k_warm.clear();
            s_warm.clear();
            let t = Instant::now();
            fill_significant(&toks, &mut k_warm, &mut s_warm);
            let e = t.elapsed().as_secs_f64() * 1e6;
            assert!(!k_warm.is_empty());
            prep_reused_s.push(e);

            let t = Instant::now();
            let mut out: Vec<Span> = Vec::with_capacity(sig / 2 + 1);
            fill_select(&spans, &result, &mut out);
            let e = t.elapsed().as_secs_f64() * 1e6;
            // Reading the fields keeps the fill live.
            assert!(out[0].end >= out[0].start);
            sel_fresh_s.push(e);
            drop(out);

            o_warm.clear();
            let t = Instant::now();
            fill_select(&spans, &result, &mut o_warm);
            let e = t.elapsed().as_secs_f64() * 1e6;
            assert!(!o_warm.is_empty());
            sel_reused_s.push(e);
        }

        let prep_fresh = median(&mut prep_fresh_s);
        let prep_reused = median(&mut prep_reused_s);
        let sel_fresh = median(&mut sel_fresh_s);
        let sel_reused = median(&mut sel_reused_s);

        if base.is_none() {
            base = Some((prep_fresh, prep_reused, sel_fresh, sel_reused));
        }
        println!(
            "{n:>10}  {prep_fresh:>12.1} {prep_reused:>12.1} {:>8.1} {sel_fresh:>15.1} {sel_reused:>12.1} {:>8.1}   {:>9}",
            prep_fresh / prep_reused,
            sel_fresh / sel_reused,
            toks.len()
        );
        println!(
            "{:>10}  prep straight off a parallel lex: {prep_after_lex:>10.1} us  \
             ({:>4.2}x the re-run)",
            "",
            prep_after_lex / prep_fresh
        );
    }

    let (pf, pr, sf, sr) = base.expect("the first size sets the baseline");
    let last = sizes[sizes.len() - 1] as f64 / sizes[0] as f64;
    println!();
    println!("growth from {} to {} tokens ({last:.0}x the data):", sizes[0], sizes[sizes.len() - 1]);
    println!("  linear would be {last:.0}x in every column.");
    println!("  baseline at the smallest size: prep {pf:.1}/{pr:.1} us, select {sf:.1}/{sr:.1} us");
    println!();
    println!("A reused column near {last:.0}x with a fresh column far above it puts the");
    println!("growth in the allocation rather than in the pass, which is what the");
    println!("phase sweep attributed to prep and select.");
}
