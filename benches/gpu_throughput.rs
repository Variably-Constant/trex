//! GPU SIMT scan throughput against the CPU engine.
//!
//! Builds a large token stream in memory, scans it for an eligible
//! pattern on the device and on the CPU, asserts the two match sets are
//! identical, and reports the wall-clock of each scan (the shared lexing
//! is measured once and excluded, so the comparison is the scan alone).
//!
//! Run with the device backend:
//!   cargo bench --features gpu --bench gpu_throughput
//!   cargo bench --features gpu --bench gpu_throughput -- --sweep   # size sweep
//!   cargo bench --features gpu --bench gpu_throughput -- --pipeline   # pipelined scan against the whole scan
//!   cargo bench --features gpu --bench gpu_throughput -- --parts      # scan over the lexed chunks in place against the whole scan
//!   cargo bench --features gpu --bench gpu_throughput -- --layers     # CPU alone, device alone over a resident corpus, and both at once
//! Without the feature, the GPU path is unavailable and the bench
//! reports that it ran CPU-only.

use std::time::Instant;

use trex::{Backend, BackendUsed, parse, scan, scan_gpu, scan_with_backend};

/// Significant-token count via a full lex, for the sweep's token column.
fn sig_token_count(bytes: &[u8]) -> usize {
    use trex::token::TokenKind;
    trex::lexer::lex(bytes).iter().filter(|t| t.kind != TokenKind::Whitespace).count()
}

fn main() {
    if std::env::args().any(|a| a == "--sweep") {
        sweep();
        phases();
        return;
    }
    if std::env::args().any(|a| a == "--phases") {
        phases();
        return;
    }
    if std::env::args().any(|a| a == "--pipeline") {
        pipeline();
        return;
    }
    if std::env::args().any(|a| a == "--parts") {
        parts();
        return;
    }
    if std::env::args().any(|a| a == "--layers") {
        layers();
        return;
    }
    if std::env::args().any(|a| a == "--split-phases") {
        split_phases();
        return;
    }
    // A large input of "word number" pairs, one a line: every pair is a
    // match for the eligible pattern below, there are many anchors for the
    // device to spread across its threads, and the pool's lexer splits the
    // input as it splits text, after a newline.
    let pairs = 2_000_000;
    let mut input = String::with_capacity(pairs * 12);
    for i in 0..pairs {
        input.push_str("tag ");
        input.push_str(&(i % 1000).to_string());
        input.push('\n');
    }
    let bytes = input.as_bytes();
    let pat = parse("\\W \\N").expect("pattern parses");

    println!("input: {} bytes, ~{} tokens, pattern \\W \\N", bytes.len(), pairs * 2);

    // CPU scan (the single-pass engine for this pattern).
    let t = Instant::now();
    let cpu = scan(&pat, bytes);
    let cpu_ms = t.elapsed().as_secs_f64() * 1000.0;

    // GPU scan; None means no device or a build without the feature.
    let t = Instant::now();
    let gpu = scan_gpu(&pat, bytes);
    let gpu_ms = t.elapsed().as_secs_f64() * 1000.0;

    println!("CPU: {:>8.2} ms  ({} matches)", cpu_ms, cpu.len());
    match gpu {
        Some(g) => {
            assert_eq!(g, cpu, "GPU and CPU match sets must be identical");
            println!("GPU: {:>8.2} ms  ({} matches)", gpu_ms, g.len());
            println!("identical: yes");
            println!("speedup (CPU / GPU): {:.2}x", cpu_ms / gpu_ms);
        }
        None => {
            println!("GPU: not available (no device, or built without --features gpu)");
            println!("ran CPU-only; rebuild with --features gpu on a CUDA host for the device path");
        }
    }
}

/// Number of independent reps per size; the reported time is the median
/// of the reps, which rejects the occasional scheduler or driver outlier
/// a single run would show.
const REPS: usize = 5;

/// Calls of the automatic router at a size before it is timed there.
const AUTO_WARM_CALLS: usize = 40;

/// Sweep the input size to find the CPU/GPU handoff and prove it is a
/// token count, not a byte length. The device runs one thread per anchor
/// (significant token), so the smallest input where the full device call
/// beats the full CPU scan by 1.5x should fall at the same token
/// count regardless of how many bytes those tokens span. The sweep runs two
/// token weights -- short tokens (many per byte) and long tokens (few per
/// byte) -- and reports the handoff in both bytes and estimated tokens: the
/// byte handoff differs several-fold, the token handoff matches. The
/// automatic router is timed beside the two once its measurements at a size
/// have settled, with the calls it placed on the CPU engine and on the split.
fn sweep() {
    let pat = parse("\\W \\N").expect("pattern parses");
    let warm = "tag 1 tag 2 tag 3 ".repeat(16);
    if scan_gpu(&pat, warm.as_bytes()).is_none() {
        println!("GPU not available; rebuild with --features gpu on a CUDA host");
        return;
    }

    for &(label, heavy) in &[("light tokens", false), ("heavy tokens", true)] {
        println!("\n=== {label} (pattern \\W \\N), median of {REPS} reps ===");
        println!(
            "{:>10}  {:>10}  {:>9}  {:>9}  {:>8}  {:>9}  {:>9}  {:>11}  winner",
            "bytes", "est tokens", "cpu ms", "gpu ms", "speedup", "auto ms", "auto/best", "cpu/split"
        );
        let mut byte_handoff: Option<usize> = None;
        let mut tok_handoff: Option<usize> = None;

        // The ladder runs from 64 pairs to 2,097,152 (16.5 MB at the light
        // weight, 75.5 MB at the heavy), so the router's floor has a row on
        // each side of it and the device's standing at the top is measured
        // rather than assumed.
        for &pairs in &[
            64usize, 256, 1024, 2048, 4096, 8192, 16_384, 65_536, 262_144, 524_288, 1_048_576,
            2_097_152,
        ] {
            let mut input = String::new();
            for i in 0..pairs {
                if heavy {
                    // Long word and long number: few tokens per byte.
                    input.push_str("supercalifragilisticword ");
                    input.push_str(&(1_000_000_000u64 + i as u64).to_string());
                } else {
                    input.push_str("tag ");
                    input.push_str(&(i % 1000).to_string());
                }
                input.push('\n');
            }
            let bytes = input.as_bytes();
            let est_tokens = sig_token_count(bytes);

            let iters = if bytes.len() < 1 << 20 { 50 } else { 10 };
            let mut cpu_samples = Vec::with_capacity(REPS);
            let mut gpu_samples = Vec::with_capacity(REPS);
            for _ in 0..REPS {
                cpu_samples.push(timed(iters, || scan(&pat, bytes).len()));
                gpu_samples.push(timed(iters, || scan_gpu(&pat, bytes).map_or(0, |m| m.len())));
            }
            let cpu_ms = median(&mut cpu_samples);
            let gpu_ms = median(&mut gpu_samples);
            let speedup = cpu_ms / gpu_ms;
            let winner = if speedup >= 1.0 { "GPU" } else { "CPU" };
            if byte_handoff.is_none() && speedup >= 1.5 {
                byte_handoff = Some(bytes.len());
                tok_handoff = Some(est_tokens);
            }

            // The automatic router, timed once its measurements at this size
            // have settled: cold, it runs both of its routes a call, and the
            // split's share moves an eighth of the way to each measured ratio.
            for _ in 0..AUTO_WARM_CALLS {
                std::hint::black_box(scan_with_backend(&pat, bytes, Backend::Auto));
            }
            let on_cpu = std::cell::Cell::new(0usize);
            let on_split = std::cell::Cell::new(0usize);
            let mut auto_samples = Vec::with_capacity(REPS);
            for _ in 0..REPS {
                auto_samples.push(timed(iters, || {
                    let (m, used) = scan_with_backend(&pat, bytes, Backend::Auto);
                    match used {
                        BackendUsed::Cpu => on_cpu.set(on_cpu.get() + 1),
                        BackendUsed::Split => on_split.set(on_split.get() + 1),
                        BackendUsed::Gpu => {}
                    }
                    m.len()
                }));
            }
            let auto_ms = median(&mut auto_samples);
            let auto_vs_best = auto_ms / cpu_ms.min(gpu_ms);
            println!(
                "{:>10}  {:>10}  {:>9.3}  {:>9.3}  {:>7.2}x  {:>9.3}  {:>8.2}x  {:>5}/{:<5}  {winner}",
                bytes.len(),
                est_tokens,
                cpu_ms,
                gpu_ms,
                speedup,
                auto_ms,
                auto_vs_best,
                on_cpu.get(),
                on_split.get()
            );
        }
        match (byte_handoff, tok_handoff) {
            (Some(b), Some(t)) => {
                println!("1.5x handoff: {b} bytes = ~{t} tokens. The byte figure differs by token weight; the device only wins this margin at the light weight real text has.");
            }
            _ => println!("no 1.5x handoff in the swept range (lex-dominated: the device ties the CPU)"),
        }
    }
}

/// Where a device scan's time goes, across sizes.
///
/// A device scan is four phases and only one of them runs on the device:
/// the lex on the cores, which writes the kind vector, the copy out, the
/// kernel with its copy back, and the host selection. Each is timed
/// separately and reported per size, so which one grows with the corpus is
/// read rather than inferred; the prep column is the host pass a token
/// stream would need to build the kind vector, zero here.
///
/// The lex is common to both engines, so the sum of the others is what the
/// device path costs over and above what the CPU path already pays.
/// Whichever of those is largest is the one worth changing.
fn phases() {
    let pat = parse("\\W \\N").expect("pattern parses");
    let warm = "tag 1 tag 2 tag 3 ".repeat(16);
    if scan_gpu(&pat, warm.as_bytes()).is_none() {
        println!("GPU not available; rebuild with --features gpu on a CUDA host");
        return;
    }
    println!("\n=== where a device scan spends itself (median of {REPS} reps) ===");
    println!("lex is common to both engines; the rest is what the device path adds");
    println!(
        "{:>10} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>8}",
        "bytes", "tokens", "lex us", "alone us", "prep us", "up us", "kern+dn", "sel us", "non-lex"
    );
    for &pairs in &[4096usize, 16_384, 65_536, 262_144, 1_048_576, 2_097_152] {
        let mut input = String::new();
        for i in 0..pairs {
            input.push_str("tag ");
            input.push_str(&(i % 1000).to_string());
            input.push('\n');
        }
        let bytes = input.as_bytes();
        // The probe must describe the scan it claims to, so its match set is
        // checked against the plain device scan before its timings are used.
        let (probe_m, _) = trex::gpu::scan_gpu_phases(&pat, bytes).expect("device runs this");
        let plain_m = scan_gpu(&pat, bytes).expect("device runs this");
        assert_eq!(probe_m, plain_m, "the phase probe must match the scan it describes");

        let mut lex = Vec::new();
        let mut prep = Vec::new();
        let mut up = Vec::new();
        let mut kern = Vec::new();
        let mut sel = Vec::new();
        let mut tokens = 0usize;
        for _ in 0..REPS {
            let (_, p) = trex::gpu::scan_gpu_phases(&pat, bytes).expect("device runs this");
            lex.push(p.lex_us);
            prep.push(p.host_prep_us);
            up.push(p.upload_us);
            kern.push(p.kernel_and_download_us);
            sel.push(p.select_us);
            tokens = p.tokens;
        }
        // The same lexer on the same bytes outside the device path, so an
        // inflation of the probe's lex phase is read rather than inferred.
        let mut alone = Vec::new();
        for _ in 0..REPS {
            let t = Instant::now();
            std::hint::black_box(trex::parallel_lex::lex_significant_parallel(bytes));
            alone.push(t.elapsed().as_secs_f64() * 1e6);
        }
        let (l, a, p, u, k, s) = (
            median(&mut lex),
            median(&mut alone),
            median(&mut prep),
            median(&mut up),
            median(&mut kern),
            median(&mut sel),
        );
        let non_lex = p + u + k + s;
        let nbytes = bytes.len();
        println!(
            "{nbytes:>10} {tokens:>9} {l:>9.1} {a:>9.1} {p:>9.1} {u:>9.1} {k:>9.1} {s:>9.1} {non_lex:>8.1}"
        );
    }
    println!("\nprep is the host pass a token stream would need for the kind vector (the lexer writes it, so zero);");
    println!("sel is the host selection; up is PCIe; kern is the device;");
    println!("alone is the probe's lexer run by itself on the same bytes.");
}

/// The pipelined device scan against the whole device scan over the phase
/// sweep's tag lines, at 2 to 16 ranges. Every pipelined result is checked
/// against the whole scan's spans before any timing; each round then times the
/// whole scan and every range count, then each of them a second time, in a
/// rotating order. A row reports the median over rounds of whole milliseconds
/// over pipelined milliseconds, the median of each of the pipeline's stage
/// times, and each arm against its own repeat, which must read 1.00x: the
/// pipelined arm overlaps host lex with device work and the whole arm does
/// not, so a neighbour moves the two by different amounts.
fn pipeline() {
    const ROUNDS: usize = 11;
    let pat = parse("\\W \\N").expect("pattern parses");
    let warm = "tag 1 tag 2 tag 3 ".repeat(16);
    if scan_gpu(&pat, warm.as_bytes()).is_none() {
        println!("GPU not available; rebuild with --features gpu on a CUDA host");
        return;
    }
    let counts = [2usize, 4, 8, 16];
    println!("\n=== the pipelined device scan against the whole device scan, median of {ROUNDS} rounds ===");
    println!(
        "{:>10} {:>7} {:>10} {:>10} {:>11} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "bytes", "ranges", "whole ms", "pipe ms", "whole/pipe", "blobs us", "lex us", "dev us", "sel us", "carry us", "overlap"
    );
    for &pairs in &[262_144usize, 1_048_576, 2_097_152] {
        let mut input = String::new();
        for i in 0..pairs {
            input.push_str("tag ");
            input.push_str(&(i % 1000).to_string());
            input.push('\n');
        }
        let bytes = input.as_bytes();
        let whole = scan_gpu(&pat, bytes).expect("device runs this");
        for &k in &counts {
            let (m, _) = trex::gpu::scan_gpu_pipelined(&pat, bytes, k).expect("device runs the pipeline");
            assert_eq!(m, whole, "the pipeline over {k} ranges must select the whole scan's spans");
        }

        // The whole scan and one arm per range count, then each of them
        // again; the stage times are read from the first run of each.
        let half = counts.len() + 1;
        let arms = 2 * half;
        let mut whole_ms = Vec::with_capacity(ROUNDS);
        let mut whole_controls = Vec::with_capacity(ROUNDS);
        let mut pipe_ms: Vec<Vec<f64>> = vec![Vec::with_capacity(ROUNDS); counts.len()];
        let mut ratios: Vec<Vec<f64>> = vec![Vec::with_capacity(ROUNDS); counts.len()];
        let mut controls: Vec<Vec<f64>> = vec![Vec::with_capacity(ROUNDS); counts.len()];
        let mut stages: Vec<Vec<trex::gpu::PipelinePhases>> = vec![Vec::with_capacity(ROUNDS); counts.len()];
        for round in 0..ROUNDS {
            let mut ms = vec![0.0f64; arms];
            for i in 0..arms {
                let arm = (i + round) % arms;
                let which = arm % half;
                let t = Instant::now();
                if which == 0 {
                    std::hint::black_box(scan_gpu(&pat, bytes));
                } else {
                    let (m, p) = trex::gpu::scan_gpu_pipelined(&pat, bytes, counts[which - 1])
                        .expect("device runs the pipeline");
                    std::hint::black_box(m);
                    // Both runs of the arm, not only the first: the stage
                    // times are the one part of this row with no arm to read
                    // them against, so their own spread across every run is
                    // what says whether they are readings.
                    stages[which - 1].push(p);
                }
                ms[arm] = t.elapsed().as_secs_f64() * 1e3;
            }
            whole_ms.push(ms[0]);
            whole_controls.push(ms[0] / ms[half]);
            for (k, ((pipe, ratio), control)) in
                pipe_ms.iter_mut().zip(ratios.iter_mut()).zip(controls.iter_mut()).enumerate()
            {
                pipe.push(ms[k + 1]);
                ratio.push(ms[0] / ms[k + 1]);
                control.push(ms[k + 1] / ms[half + k + 1]);
            }
        }
        let whole_med = median(&mut whole_ms);
        let whole_control = median(&mut whole_controls);
        for (k, timed) in stages.iter().enumerate() {
            let stage = |f: fn(&trex::gpu::PipelinePhases) -> f64| {
                let mut v: Vec<f64> = timed.iter().map(f).collect();
                median(&mut v)
            };
            // The device stage's own largest reading over its smallest. It is
            // the column a tenant on the card moves, and the only one no arm
            // can speak for: a spread far from one says the card was shared
            // while this row was timed, whatever the wall-clock arms read.
            let spread = |f: fn(&trex::gpu::PipelinePhases) -> f64| {
                let mut v: Vec<f64> = timed.iter().map(f).collect();
                v.sort_by(|a, b| a.partial_cmp(b).expect("a measured time is never NaN"));
                match (v.first(), v.last()) {
                    (Some(&lo), Some(&hi)) if lo > 0.0 => hi / lo,
                    _ => f64::INFINITY,
                }
            };
            let ranges = timed.first().map_or(0, |p| p.partitions);
            let (b, l, d, s, c, o) = (
                stage(|p| p.blobs_us),
                stage(|p| p.lex_us),
                stage(|p| p.device_us),
                stage(|p| p.select_us),
                stage(|p| p.carry_us),
                stage(|p| p.overlap_us()),
            );
            println!(
                "{:>10} {ranges:>7} {whole_med:>10.3} {:>10.3} {:>10.3}x {b:>9.1} {l:>9.1} {d:>9.1} {s:>9.1} {c:>9.1} {o:>9.1}   control whole {whole_control:.3}x pipe {:.3}x dev spread {:.2}x",
                bytes.len(),
                median(&mut pipe_ms[k]),
                median(&mut ratios[k]),
                median(&mut controls[k]),
                spread(|p| p.device_us),
            );
        }
    }
    println!("\nwhole/pipe above 1 is the pipeline faster; overlap is the stages' busy time past the wall time; each control is an arm against its own repeat, and dev spread is the device stage's own largest reading over its smallest, which is what a tenant on the card moves.");
}

/// The device scan over the lexed chunks in place against the whole device
/// scan, over the phase sweep's tag lines. The chunk scan's spans are checked
/// against the whole scan's before any timing; each round then times the whole
/// scan, the chunk scan and the whole scan again, in a rotating order. A row
/// reports the median over rounds of whole milliseconds over chunk-scan
/// milliseconds with the smallest and largest, and the whole scan against its
/// repeat, which must read 1.00x.
fn parts() {
    const ROUNDS: usize = 21;
    let pat = parse("\\W \\N").expect("pattern parses");
    let warm = "tag 1 tag 2 tag 3 ".repeat(16);
    if scan_gpu(&pat, warm.as_bytes()).is_none() {
        println!("GPU not available; rebuild with --features gpu on a CUDA host");
        return;
    }
    println!("\n=== the device scan over the lexed chunks in place against the whole device scan, {ROUNDS} rounds ===");
    println!(
        "{:>10} {:>10} {:>10} {:>12} {:>17} {:>9}",
        "bytes", "whole ms", "parts ms", "whole/parts", "spread", "control"
    );
    for &pairs in &[131_072usize, 524_288, 1_048_576, 2_097_152] {
        let mut input = String::new();
        for i in 0..pairs {
            input.push_str("tag ");
            input.push_str(&(i % 1000).to_string());
            input.push('\n');
        }
        let bytes = input.as_bytes();
        let whole = scan_gpu(&pat, bytes).expect("device runs this");
        let chunked = trex::gpu::scan_gpu_parts(&pat, bytes).expect("device runs the parts scan");
        assert_eq!(chunked, whole, "the parts scan must select the whole scan's spans");

        let mut whole_ms = Vec::with_capacity(ROUNDS);
        let mut parts_ms = Vec::with_capacity(ROUNDS);
        let mut ratios = Vec::with_capacity(ROUNDS);
        let mut controls = Vec::with_capacity(ROUNDS);
        for round in 0..ROUNDS {
            let mut ms = [0.0f64; 3];
            for i in 0..3 {
                let arm = (i + round) % 3;
                let t = Instant::now();
                if arm == 1 {
                    std::hint::black_box(trex::gpu::scan_gpu_parts(&pat, bytes));
                } else {
                    std::hint::black_box(scan_gpu(&pat, bytes));
                }
                ms[arm] = t.elapsed().as_secs_f64() * 1e3;
            }
            whole_ms.push(ms[0]);
            parts_ms.push(ms[1]);
            ratios.push(ms[0] / ms[1]);
            controls.push(ms[0] / ms[2]);
        }
        let lo = ratios.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = ratios.iter().copied().fold(0.0, f64::max);
        println!(
            "{:>10} {:>10.3} {:>10.3} {:>11.3}x {:>8.3}..{:<8.3} {:>8.3}x",
            bytes.len(),
            median(&mut whole_ms),
            median(&mut parts_ms),
            median(&mut ratios),
            lo,
            hi,
            median(&mut controls)
        );
    }
    println!("\nwhole/parts above 1 is the parts scan faster; control is the whole scan against its repeat.");
}

/// The three layers over one corpus: the CPU engine alone, the device alone
/// with the corpus's kinds uploaded once and kept there, and the host's cores
/// and the device splitting one lex between them. Each layer's spans are
/// checked against the CPU engine's before any timing; every round then times
/// the CPU engine, the resident device scan, the split, and the CPU engine
/// again as the control, in a rotating order. A row reports each layer's
/// median milliseconds a scan, the CPU engine's time over each, the control,
/// and the one-off lex and upload the resident layer pays before its first
/// scan.
fn layers() {
    const ROUNDS: usize = 11;
    let pat = parse("\\W \\N").expect("pattern parses");
    let warm = "tag 1 tag 2 tag 3 ".repeat(16);
    if scan_gpu(&pat, warm.as_bytes()).is_none() {
        println!("GPU not available; rebuild with --features gpu on a CUDA host");
        return;
    }
    println!(
        "\n=== the CPU engine alone, the device alone over a resident corpus, and both at once, median of {ROUNDS} rounds ==="
    );
    // The host's share of the resident split, in per mille of the anchors.
    // The sweep peaks here: against the device alone, 1.046x at 16.5 MB and
    // 1.162x at 33 MB, lower at every other share from 500 to 950.
    const HOST_PER_MILLE: u32 = 700;
    println!(
        "{:>10} {:>9} {:>12} {:>9} {:>13} {:>13} {:>10} {:>14} {:>9} {:>10}",
        "bytes",
        "cpu ms",
        "resident ms",
        "split ms",
        "res split ms",
        "cpu/resident",
        "cpu/split",
        "cpu/res split",
        "control",
        "upload ms"
    );
    for &pairs in &[262_144usize, 1_048_576, 2_097_152, 4_194_304] {
        let mut input = String::new();
        for i in 0..pairs {
            input.push_str("tag ");
            input.push_str(&(i % 1000).to_string());
            input.push('\n');
        }
        let bytes = input.as_bytes();
        let want = scan(&pat, bytes);
        let t = Instant::now();
        let held = trex::gpu::GpuTokens::upload(bytes).expect("the device holds the corpus");
        let upload_ms = t.elapsed().as_secs_f64() * 1e3;
        assert_eq!(
            held.scan(&pat).expect("the resident scan runs"),
            want,
            "the resident scan must select the CPU engine's spans"
        );
        assert_eq!(
            trex::gpu::scan_gpu_split(&pat, bytes).expect("the split runs"),
            want,
            "the split must select the CPU engine's spans"
        );
        assert_eq!(
            held.scan_split(&pat, HOST_PER_MILLE).expect("the resident split runs"),
            want,
            "the resident split must select the CPU engine's spans"
        );

        let mut cpu_ms = Vec::with_capacity(ROUNDS);
        let mut resident_ms = Vec::with_capacity(ROUNDS);
        let mut split_ms = Vec::with_capacity(ROUNDS);
        let mut res_split_ms = Vec::with_capacity(ROUNDS);
        let mut resident_ratio = Vec::with_capacity(ROUNDS);
        let mut split_ratio = Vec::with_capacity(ROUNDS);
        let mut res_split_ratio = Vec::with_capacity(ROUNDS);
        let mut controls = Vec::with_capacity(ROUNDS);
        for round in 0..ROUNDS {
            let mut ms = [0.0f64; 5];
            for i in 0..5 {
                let arm = (i + round) % 5;
                let t = Instant::now();
                match arm {
                    1 => std::hint::black_box(held.scan(&pat)),
                    2 => std::hint::black_box(trex::gpu::scan_gpu_split(&pat, bytes)),
                    3 => std::hint::black_box(held.scan_split(&pat, HOST_PER_MILLE)),
                    _ => std::hint::black_box(Some(scan(&pat, bytes))),
                };
                ms[arm] = t.elapsed().as_secs_f64() * 1e3;
            }
            cpu_ms.push(ms[0]);
            resident_ms.push(ms[1]);
            split_ms.push(ms[2]);
            res_split_ms.push(ms[3]);
            resident_ratio.push(ms[0] / ms[1]);
            split_ratio.push(ms[0] / ms[2]);
            res_split_ratio.push(ms[0] / ms[3]);
            controls.push(ms[0] / ms[4]);
        }
        println!(
            "{:>10} {:>9.3} {:>12.3} {:>9.3} {:>13.3} {:>12.3}x {:>9.3}x {:>13.3}x {:>8.3}x {:>10.3}",
            bytes.len(),
            median(&mut cpu_ms),
            median(&mut resident_ms),
            median(&mut split_ms),
            median(&mut res_split_ms),
            median(&mut resident_ratio),
            median(&mut split_ratio),
            median(&mut res_split_ratio),
            median(&mut controls),
            upload_ms
        );
    }
    println!(
        "\nevery ratio above 1 is that layer faster than the CPU engine; res split is the split over the resident corpus, which lexes and uploads nothing; control is the CPU engine against its own repeat; upload is the lex and upload the resident layers pay once."
    );
    share_sweep();
}

/// Where a resident split spends itself: the per-anchor ends the host and the
/// device compute at once, and the host's selection over the joined table. The
/// spans are checked against the resident scan's before any phase is read.
///
/// Both phases are timed inside one call, so they share that call's wall time
/// as their denominator: a neighbour's load inflates them together and the
/// share stays readable without a quiet box. The microseconds are not: they
/// are what a pre-registered band is written in, and a neighbour inflates
/// them. So each round times the split twice and reports the median of the
/// per-round paired ratios as a control. A control near 1.000 says the box
/// held still across a round and the microseconds beside it are the split's;
/// a control that spreads says they are the box's and the row is not read.
fn split_phases() {
    const ROUNDS: usize = 11;
    const HOST_PER_MILLE: u32 = 700;

    /// One row: the split's phases for `pat_src` over a corpus the device
    /// already holds, its spans checked against the resident scan before any
    /// phase is read. The matches and the matches per anchor sit beside the
    /// phases because the selection skips past a match and steps one slot over
    /// a miss, so what it costs depends on how many anchors it visits.
    fn row(
        pat_src: &str,
        held: &trex::gpu::GpuTokens,
        anchors: usize,
        nbytes: usize,
        rounds: usize,
        share_per_mille: u32,
    ) {
        let pat = parse(pat_src).expect("pattern parses");
        let want = held.scan(&pat).expect("the resident scan runs");
        let (spans, _) = held
            .scan_split_phases(&pat, share_per_mille)
            .expect("the resident split reports its phases");
        assert_eq!(spans, want, "the timed split must select the resident scan's spans");
        let matches = want.len();
        let mut ends = Vec::with_capacity(rounds);
        let mut sel = Vec::with_capacity(rounds);
        let mut wall = Vec::with_capacity(rounds);
        let mut share = Vec::with_capacity(rounds);
        let mut ends_ctl = Vec::with_capacity(rounds);
        let mut sel_ctl = Vec::with_capacity(rounds);
        let phase = || {
            held.scan_split_phases(&pat, share_per_mille)
                .expect("the resident split reports its phases")
                .1
        };
        for _ in 0..rounds {
            let p = phase();
            // The same call again, adjacent in time, so each phase is read
            // against a copy of itself rather than against the box.
            let c = phase();
            ends.push(p.ends_us);
            sel.push(p.select_us);
            wall.push(p.wall_us);
            share.push(p.select_us / p.wall_us);
            ends_ctl.push(c.ends_us / p.ends_us);
            sel_ctl.push(c.select_us / p.select_us);
        }
        let per = if anchors == 0 { 0.0 } else { matches as f64 / anchors as f64 };
        // The selection walks every anchor and writes every match, so its cost
        // per anchor and per match is what a row is compared across corpora by.
        let sel_us = median(&mut sel);
        let ns_per_anchor = if anchors == 0 { 0.0 } else { sel_us * 1000.0 / anchors as f64 };
        println!(
            "{pat_src:>18} {nbytes:>10} {anchors:>9} {matches:>9} {per:>11.4} {:>9.1} {sel_us:>10.1} {:>9.1} {:>12.3}x {:>9.3}x {:>9.3}x {ns_per_anchor:>12.3}",
            median(&mut ends),
            median(&mut wall),
            median(&mut share),
            median(&mut ends_ctl),
            median(&mut sel_ctl)
        );
    }

    let pat = parse("\\W \\N").expect("pattern parses");
    let warm = "tag 1 tag 2 tag 3 ".repeat(16);
    if scan_gpu(&pat, warm.as_bytes()).is_none() {
        println!("GPU not available; rebuild with --features gpu on a CUDA host");
        return;
    }
    println!(
        "\n=== where a resident split spends itself at {HOST_PER_MILLE} per mille, median of {ROUNDS} rounds ==="
    );
    println!(
        "{:>18} {:>10} {:>9} {:>9} {:>11} {:>9} {:>10} {:>9} {:>13} {:>10} {:>10} {:>12}",
        "pattern", "bytes", "anchors", "matches", "per anchor", "ends us", "select us", "wall us",
        "select share", "ends ctl", "select ctl", "select ns/a"
    );
    for &pairs in &[262_144usize, 1_048_576, 2_097_152, 4_194_304] {
        let mut input = String::new();
        for i in 0..pairs {
            input.push_str("tag ");
            input.push_str(&(i % 1000).to_string());
            input.push('\n');
        }
        let bytes = input.as_bytes();
        let held = trex::gpu::GpuTokens::upload(bytes).expect("the device holds the corpus");
        row("\\W \\N", &held, sig_token_count(bytes), bytes.len(), ROUNDS, HOST_PER_MILLE);
    }
    // A real corpus under two densities over the same bytes and the same
    // anchors: on train_corpus2 `\W \N` matches about one anchor in eighty-five
    // and `\W \W` about one in three. The pair reads whether the selection's
    // cost follows the matches it writes or the anchors it walks, with the
    // corpus, the upload and the anchor count all held fixed.
    if let Some(path) = std::env::args().skip(1).find(|a| !a.starts_with("--")) {
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("cannot read {path}: {e}");
                std::process::exit(1);
            }
        };
        let held = trex::gpu::GpuTokens::upload(&bytes).expect("the device holds the corpus");
        let anchors = sig_token_count(&bytes);
        for src in ["\\W \\N", "\\W \\W"] {
            row(src, &held, anchors, bytes.len(), ROUNDS, HOST_PER_MILLE);
        }
    }
    println!(
        "\nselect share is the selection over its own call's wall time, so it reads the same on a loaded box. per anchor is the matches over the anchors: the walk skips past a match and steps one slot over a miss, so a sparse pattern visits nearly every slot to write almost nothing. the ctl columns are each phase against a second copy of itself taken in the same round: near 1.000 the microseconds beside them are the split's, spread they are the box's and the row is not read. select ns/a is the selection over the anchors it walked."
    );
}

/// The host's share of the resident split, swept over the two largest corpora,
/// so the share where both sides finish together is read rather than assumed.
/// Every share's spans are checked against the resident scan's first; each
/// round then times the resident scan and every share once, in a rotating
/// order. A row reports the median milliseconds a scan and the resident scan's
/// time over the split's, which is above 1 only where adding the host pays.
fn share_sweep() {
    const ROUNDS: usize = 11;
    // The sweep at ca74fbf crossed 1.000x at 320 per mille and still climbed
    // at its top share, 500 reading 1.041x at 16.5 MB and 1.071x at 33 MB, so
    // the ladder carries on into the half of the anchors above it.
    const SHARES: [u32; 6] = [500, 600, 700, 800, 900, 950];
    let pat = parse("\\W \\N").expect("pattern parses");
    println!("\n=== the host's share of the resident split, median of {ROUNDS} rounds ===");
    println!(
        "{:>10} {:>12} {:>9} {:>12} {:>16}",
        "bytes", "resident ms", "share", "split ms", "resident/split"
    );
    for &pairs in &[2_097_152usize, 4_194_304] {
        let mut input = String::new();
        for i in 0..pairs {
            input.push_str("tag ");
            input.push_str(&(i % 1000).to_string());
            input.push('\n');
        }
        let bytes = input.as_bytes();
        let held = trex::gpu::GpuTokens::upload(bytes).expect("the device holds the corpus");
        let want = held.scan(&pat).expect("the resident scan runs");
        for &share in &SHARES {
            assert_eq!(
                held.scan_split(&pat, share).expect("the resident split runs"),
                want,
                "the resident split at {share} per mille must select the resident scan's spans"
            );
        }

        let arms = SHARES.len() + 1;
        let mut resident_ms = Vec::with_capacity(ROUNDS);
        let mut split_ms: Vec<Vec<f64>> = vec![Vec::with_capacity(ROUNDS); SHARES.len()];
        let mut ratios: Vec<Vec<f64>> = vec![Vec::with_capacity(ROUNDS); SHARES.len()];
        for round in 0..ROUNDS {
            let mut ms = vec![0.0f64; arms];
            for i in 0..arms {
                let arm = (i + round) % arms;
                let t = Instant::now();
                if arm == 0 {
                    std::hint::black_box(held.scan(&pat));
                } else {
                    std::hint::black_box(held.scan_split(&pat, SHARES[arm - 1]));
                }
                ms[arm] = t.elapsed().as_secs_f64() * 1e3;
            }
            resident_ms.push(ms[0]);
            for (k, (split, ratio)) in split_ms.iter_mut().zip(ratios.iter_mut()).enumerate() {
                split.push(ms[k + 1]);
                ratio.push(ms[0] / ms[k + 1]);
            }
        }
        let resident = median(&mut resident_ms);
        for (k, &share) in SHARES.iter().enumerate() {
            println!(
                "{:>10} {resident:>12.3} {share:>9} {:>12.3} {:>15.3}x",
                bytes.len(),
                median(&mut split_ms[k]),
                median(&mut ratios[k])
            );
        }
    }
    println!(
        "\nresident/split above 1 is the split faster than the device alone over the same resident corpus."
    );
}

/// Median of the samples (sorted in place). Robust to the single slow rep
/// a mean would be dragged by.
fn median(xs: &mut [f64]) -> f64 {
    xs.sort_by(|a, b| a.partial_cmp(b).expect("no NaN timings"));
    let n = xs.len();
    if n == 0 {
        return 0.0;
    }
    if n % 2 == 1 { xs[n / 2] } else { (xs[n / 2 - 1] + xs[n / 2]) / 2.0 }
}

/// Average wall-clock milliseconds of `f` over `iters` runs, after one
/// warm call so first-touch allocation and the device context are not
/// charged to the measurement.
fn timed<F: Fn() -> usize>(iters: usize, f: F) -> f64 {
    std::hint::black_box(f());
    let t = Instant::now();
    let mut acc = 0usize;
    for _ in 0..iters {
        acc = acc.wrapping_add(f());
    }
    std::hint::black_box(acc);
    t.elapsed().as_secs_f64() * 1000.0 / iters as f64
}
