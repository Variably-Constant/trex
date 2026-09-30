//! Scheduler engagement probe: does the parallel scan's `JobPlan`
//! (K_outer = 0, LeafShape::PortCompute, the batch size the scan uses)
//! actually run leaves on more than one worker thread, or does it collapse
//! to inline serial execution? Counts the distinct worker thread ids that
//! run a leaf, and reports wall vs summed-busy time as a second check.
//!
//!   cargo run --release --example sched_probe

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Instant;

use flynnel::JobPlan;
use flynnel::sched::par_iter::for_each_chunk_indexed_min_leaf;

fn main() {
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);

    for &m in &[2048usize, 65_536, 2_000_000] {
        let mut v = vec![0u64; m];
        let threads: Mutex<HashSet<String>> = Mutex::new(HashSet::new());
        let busy_ns = Mutex::new(0u128);

        // The exact plan shape the scan uses.
        let min_leaf = m.div_ceil(cores * 4).max(64);
        let plan = JobPlan::new(0, m as u32)
            .with_leaf_shape(flynnel::LeafShape::PortCompute)
            .with_estimated_per_item_ns(256);

        let wall = Instant::now();
        for_each_chunk_indexed_min_leaf(&plan, &mut v, min_leaf, |start, slots| {
            let t = Instant::now();
            let id = format!("{:?}", std::thread::current().id());
            // Real per-leaf work so a leaf is not instant (mirrors the
            // per-anchor match cost order of magnitude).
            for (i, s) in slots.iter_mut().enumerate() {
                let mut x = (start + i) as u64;
                for _ in 0..200 {
                    x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                }
                *s = x;
            }
            threads.lock().unwrap().insert(id);
            *busy_ns.lock().unwrap() += t.elapsed().as_nanos();
        });
        let wall_ns = wall.elapsed().as_nanos();
        let busy = *busy_ns.lock().unwrap();
        let n = threads.lock().unwrap().len();

        println!(
            "m={m:>8}  min_leaf={min_leaf:>6}  distinct worker threads={n:>2}  busy/wall={:.2}x  {}",
            busy as f64 / wall_ns as f64,
            if n > 1 { "POOL ENGAGED" } else { "INLINE (serial) - pool NOT engaged" }
        );
    }
    println!("cores={cores}");
}
