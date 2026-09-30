//! What a resident split's host share costs, three ways, with the device's
//! half held fixed: the kernel's bit-parallel automaton on the host across
//! the cores, which is what the split runs today; the single-pass engine
//! filling the same table per anchor across the cores; and that engine as one
//! forward walk over the whole host prefix.
//!
//! The port arm needs the `gpu` feature to be compiled in, but no device: it
//! is the host's own run of the kernel's loop.
//!
//! The three differ in what they compute, not only in how fast. The port and
//! the per-anchor engine fill an end at every anchor of the share. The
//! forward walk fills only the matches a leftmost, non-overlapping selection
//! would take, which is sound because that selection lands on exactly those
//! anchors - and is why it can be cheaper than either. It is also sequential,
//! where the other two run across the cores, which is what this run is here
//! to price against each other.
//!
//! Every arm's spans are checked against the CPU engine's before any arm is
//! timed, and each arm is run against a second copy of itself in the same
//! round, so a neighbour shows in the control rather than in the ratio.
//!
//! No device is used, so this needs no CUDA host: it prices the host's share
//! alone, which is the half the split can choose how to compute.
//!
//! Run: `cargo run --release --example host_share_arms -- [corpus ...]`.

use std::hint::black_box;
use std::path::Path;
use std::time::Instant;

use trex::engine::Span;
use trex::nfa::{Compiled, KindSpans, anchor_ends_into, compile_pattern, walk_prefix_into};

const ROUNDS: usize = 11;
/// The share the split runs at, measured twice as its balance point.
const HOST_PER_MILLE: usize = 700;

fn tag_pairs(pairs: usize) -> Vec<u8> {
    let mut input = String::new();
    for i in 0..pairs {
        input.push_str("tag ");
        input.push_str(&(i % 1000).to_string());
        input.push('\n');
    }
    input.into_bytes()
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// The leftmost, non-overlapping spans over a per-anchor ends table.
///
/// An anchor with no match carries a sentinel at or below its own index, and
/// which one depends on who filled the slot: the port writes -1, the engine
/// writes 0. Both are answered by comparing as the stored `i32` - widening
/// first would read -1 as a vast end and index past the spans.
fn select(spans: &[(u32, u32)], ends: &[i32]) -> Vec<Span> {
    let mut out = Vec::new();
    let mut a = 0usize;
    while a < ends.len() {
        if ends[a] > a as i32 {
            let e = ends[a] as usize;
            out.push(Span { start: spans[a].0, end: spans[e - 1].1 });
            a = e;
        } else {
            a += 1;
        }
    }
    out
}

/// Which way the host's share of the ends table is filled.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Arm {
    /// The kernel's bit-parallel automaton on the host, across the cores:
    /// what the split runs today.
    Port,
    /// The single-pass engine per anchor, across the cores.
    EnginePerAnchor,
    /// The single-pass engine as one forward walk over the whole share.
    EngineWalk,
}

/// Milliseconds to fill `out[..mid]` the way `arm` fills it, the rest of the
/// table left as `device` holds it.
// The eight are one arm's whole input: which arm, the pattern in both the
// forms its arms need, the stream it walks, where the host's share ends, and
// the device's table in and the filled table out.
#[allow(clippy::too_many_arguments)]
fn host_share(
    arm: Arm,
    pat: &trex::ast::Pattern,
    kinds: &[u32],
    prog: &Compiled,
    stream: &KindSpans<'_>,
    mid: usize,
    device: &[i32],
    out: &mut Vec<i32>,
) -> f64 {
    out.clear();
    out.extend_from_slice(device);
    let t = Instant::now();
    match arm {
        Arm::Port => {
            assert!(
                trex::gpu::port_anchor_ends(pat, kinds, 0, &mut out[..mid]),
                "the port takes a pattern the split would give it"
            );
        }
        Arm::EnginePerAnchor => {
            // Across the cores, as the split runs its port: each leaf owns a
            // run of anchors and writes only its own.
            use flynnel::JobPlan;
            use flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf;
            let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
            let min_leaf = mid.div_ceil(cores * 4).max(64);
            let plan = JobPlan::new(0, mid.min(u32::MAX as usize) as u32)
                .with_leaf_shape(flynnel::LeafShape::PortCompute);
            for_each_chunk_indexed_min_leaf(&plan, &mut out[..mid], min_leaf, |start, slots| {
                anchor_ends_into(prog, &[], stream, start, slots);
            });
        }
        Arm::EngineWalk => {
            for slot in &mut out[..mid] {
                *slot = 0;
            }
            walk_prefix_into(prog, &[], stream, mid, &mut out[..mid]);
        }
    }
    let ms = t.elapsed().as_secs_f64() * 1e3;
    black_box(out.len());
    ms
}

fn main() {
    let mut cells: Vec<(String, Vec<u8>)> = vec![
        ("tag-lines-262144".to_string(), tag_pairs(262_144)),
        ("tag-lines-2097152".to_string(), tag_pairs(2_097_152)),
    ];
    for path in std::env::args().skip(1) {
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("cannot read {path}: {e}");
                std::process::exit(1);
            }
        };
        let stem = match Path::new(&path).file_stem() {
            Some(s) => s.to_string_lossy().into_owned(),
            None => {
                eprintln!("{path} names no file");
                std::process::exit(1);
            }
        };
        cells.push((stem, bytes));
    }

    println!(
        "=== a resident split's host share at {HOST_PER_MILLE} per mille, three ways, median of {ROUNDS} rounds ==="
    );
    println!(
        "{:>22} {:>11} {:>8} {:>10} {:>8} {:>13} {:>9} {:>13} {:>10} {:>9} {:>9} {:>9}",
        "cell", "pattern", "anchors", "matches", "port ms", "per anchor ms", "walk ms",
        "anchor/port", "walk/port", "ctl port", "ctl anchor", "ctl walk"
    );
    for (cell, input) in &cells {
        let (kinds, spans) = trex::parallel_lex::lex_significant_parallel(input);
        let stream = KindSpans::new(&kinds, &spans);
        let n = kinds.len();
        if n < 2 {
            println!("{cell:>22} has {n} anchors");
            continue;
        }
        let mid = (n * HOST_PER_MILLE / 1000).clamp(1, n - 1);
        for src in ["\\W \\N", "\\W \\W"] {
            let pat = trex::parse(src).expect("pattern parses");
            let prog = compile_pattern(&pat).expect("the engine takes the pattern");
            // The device's half, computed once on the host and reused by every
            // arm, so what is timed is the share and not the other side. The
            // engine's no-match sentinel is 0, which the selection reads the
            // same as the port's -1.
            let mut device = vec![0i32; n];
            anchor_ends_into(&prog, &[], &stream, mid, &mut device[mid..]);
            let want = trex::engine::scan(&pat, input);
            let mut out = Vec::with_capacity(n);
            for arm in [Arm::Port, Arm::EnginePerAnchor, Arm::EngineWalk] {
                host_share(arm, &pat, &kinds, &prog, &stream, mid, &device, &mut out);
                assert_eq!(select(&spans, &out), want, "{cell} {src}: an arm's spans are not the engine's");
            }
            let matches = want.len();
            let (mut port, mut per_anchor, mut walk) = (Vec::new(), Vec::new(), Vec::new());
            let (mut anchor_ratio, mut walk_ratio) = (Vec::new(), Vec::new());
            let (mut port_ctl, mut anchor_ctl, mut walk_ctl) = (Vec::new(), Vec::new(), Vec::new());
            for round in 0..ROUNDS {
                // Arms 3 to 5 rerun arms 0 to 2, rotated, so each is read
                // against its own repeat taken in the same round.
                let mut ms = [0.0f64; 6];
                for i in 0..6 {
                    let arm = (i + round) % 6;
                    let which = match arm % 3 {
                        0 => Arm::Port,
                        1 => Arm::EnginePerAnchor,
                        _ => Arm::EngineWalk,
                    };
                    ms[arm] = host_share(which, &pat, &kinds, &prog, &stream, mid, &device, &mut out);
                }
                port.push(ms[0]);
                per_anchor.push(ms[1]);
                walk.push(ms[2]);
                anchor_ratio.push(ms[1] / ms[0]);
                walk_ratio.push(ms[2] / ms[0]);
                port_ctl.push(ms[3] / ms[0]);
                anchor_ctl.push(ms[4] / ms[1]);
                walk_ctl.push(ms[5] / ms[2]);
            }
            println!(
                "{cell:>22} {src:>11} {mid:>8} {matches:>10} {:>8.3} {:>13.3} {:>9.3} {:>12.3}x {:>9.3}x {:>8.3}x {:>8.3}x {:>8.3}x",
                median(&mut port),
                median(&mut per_anchor),
                median(&mut walk),
                median(&mut anchor_ratio),
                median(&mut walk_ratio),
                median(&mut port_ctl),
                median(&mut anchor_ctl),
                median(&mut walk_ctl)
            );
        }
    }
    println!(
        "\nboth ratios are against the port, the bit-parallel automaton the split runs today across the cores; below 1 is faster than it. per anchor is the single-pass engine filling the same table across the cores; walk is that engine as one forward walk on one core, writing only the matches the selection takes. matches is over the whole input, not the share."
    );
}
