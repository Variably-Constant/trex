//! CPU + GPU tandem dispatch (the default `tandem` feature, which implies `gpu`).
//!
//! A batch of inputs is split across the device and the cores and
//! scanned at the same time. Each input is scanned independently, so the
//! result for input `i` equals [`crate::scan`] on that input; the win is
//! that the device works on its share of the batch while the cores work
//! on theirs, joined through the scheduler's hybrid CPU + accelerator
//! join. The device half owns its inputs (the join runs it on a separate
//! thread), so the split moves the batch rather than copying it.
//!
//! This is the batching form of CPU + GPU overlap: with independent
//! inputs there is no seam between a device chunk and a core chunk, so
//! the tandem result is exactly the per-input scan with no
//! reconciliation, while both engines stay busy.

use std::time::Instant;

use crate::ast::Pattern;
use crate::engine::{Span, scan};
use crate::gpu::scan_gpu;

/// The device's share of a batch, balanced by measured per-input speed:
/// the engine that scans an input faster takes proportionally more of
/// the batch so both halves finish together. The device is warmed first
/// so its one-time JIT and context setup do not skew the estimate. With
/// no device present, the device timing collapses to the CPU fallback's,
/// the split lands near half, and both halves run on the cores anyway.
fn balanced_device_share(pattern: &Pattern, probe: &[u8], n: usize) -> usize {
    use std::hint::black_box;
    // black_box keeps the optimizer from deleting the timed scans.
    black_box(scan_gpu(pattern, probe)); // warm the device (JIT + context)
    let t = Instant::now();
    black_box(scan(pattern, probe));
    let cpu_ns = t.elapsed().as_nanos().max(1);
    let t = Instant::now();
    black_box(scan_gpu(pattern, probe).unwrap_or_else(|| scan(pattern, probe)));
    let gpu_ns = t.elapsed().as_nanos().max(1);
    // Balance: gpu_count * gpu_ns == (n - gpu_count) * cpu_ns, so the
    // device share grows with how much faster the device is.
    let gpu_count = (n as u128 * cpu_ns) / (cpu_ns + gpu_ns);
    (gpu_count as usize).clamp(1, n)
}

/// Scan a batch of inputs across the GPU and the CPU concurrently,
/// returning one match list per input in input order. Each list equals
/// [`crate::scan`] on the corresponding input; the device and the cores
/// run their shares at the same time.
#[must_use]
pub fn scan_batch_tandem(pattern: &Pattern, mut inputs: Vec<Vec<u8>>) -> Vec<Vec<Span>> {
    use flynnel::{JobPlan, join_hybrid};

    let n = inputs.len();
    if n == 0 {
        return Vec::new();
    }
    let gpu_count = balanced_device_share(pattern, &inputs[0], n);
    // Split the batch by move: the device half is owned by its closure
    // (the hybrid join runs it on its own thread), the core half stays
    // borrowed on the calling thread.
    let cpu_share: Vec<Vec<u8>> = inputs.split_off(gpu_count);
    let gpu_share: Vec<Vec<u8>> = inputs;
    let pat_for_gpu = pattern.clone();
    // `join_hybrid` reads one thing from the plan, `pick_backend`, and runs the
    // core half on the calling thread. A leaf shape sets fan-out and deque
    // knobs, and there is no fan-out here for them to reach, so setting one
    // would read as a routing decision that does not route.
    //
    // The field that would matter is the backend hint: with none set,
    // `pick_backend` falls through to the CPU backend rather than reporting
    // that it found no device.
    let plan = JobPlan::new(0, n as u32);

    // `join_hybrid(plan, cpu_work, gpu_work)`: the first closure runs on
    // the calling thread (it may borrow), the second is dispatched to a
    // worker thread (it must be `Send + 'static`, so it owns its share).
    let (cpu_res, gpu_res) = join_hybrid(
        &plan,
        // Core half: scan the borrowed rest on the calling thread.
        || -> Vec<Vec<Span>> { cpu_share.iter().map(|inp| scan(pattern, inp)).collect() },
        // Device half: scan each owned input on the device concurrently,
        // falling back to the CPU engine for any input outside the
        // device subset.
        move || -> Vec<Vec<Span>> {
            gpu_share
                .iter()
                .map(|inp| scan_gpu(&pat_for_gpu, inp).unwrap_or_else(|| scan(&pat_for_gpu, inp)))
                .collect()
        },
    );

    // The device half held inputs [0, gpu_count); the core half held the
    // rest. Concatenating restores input order.
    let mut out = gpu_res;
    out.extend(cpu_res);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    #[test]
    fn tandem_batch_equals_per_input_scan() {
        let pat = parse("\\W \\N").expect("pattern parses");
        let inputs: Vec<Vec<u8>> = (0..50)
            .map(|i| format!("row {i} value {} end {i}", i * 3).into_bytes())
            .collect();
        let want: Vec<Vec<Span>> = inputs.iter().map(|inp| scan(&pat, inp)).collect();
        let got = scan_batch_tandem(&pat, inputs);
        assert_eq!(got, want, "tandem batch must equal the per-input scan, in order");
    }

    #[test]
    fn empty_batch_is_empty() {
        let pat = parse("\\N").expect("pattern parses");
        assert!(scan_batch_tandem(&pat, Vec::new()).is_empty());
    }

    #[test]
    fn single_input_batch() {
        let pat = parse("\\N").expect("pattern parses");
        let got = scan_batch_tandem(&pat, vec![b"a 1 b 2".to_vec()]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0], scan(&pat, b"a 1 b 2"));
    }
}
