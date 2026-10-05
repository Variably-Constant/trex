//! The standard output as a scan's reports write it, with the bytes written
//! counted, so `--stats` can say how many were printed.

use std::sync::atomic::{AtomicU64, Ordering};

static PRINTED: AtomicU64 = AtomicU64::new(0);

/// Print `s` and a newline.
pub fn line(s: &str) {
    println!("{s}");
    PRINTED.fetch_add(s.len() as u64 + 1, Ordering::Relaxed);
}

/// How many bytes the reports have printed so far.
pub fn printed() -> u64 {
    PRINTED.load(Ordering::Relaxed)
}
