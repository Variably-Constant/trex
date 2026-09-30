//! Emit one despaced slice, its true boundaries, and trex's own cuts and score.
//!
//! The comparison against other segmenters is only valid if every one of them
//! sees the same bytes and is scored against the same truth, so the input and
//! the answer key are written out here rather than reconstructed on the other
//! side. `trex_f1` is printed so an external scorer can be checked against this
//! one on identical cuts before it is trusted on anyone else's.

use trex::seam::{self, SeamConfig};

fn die(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1)
}

fn write_offsets(outdir: &str, name: &str, v: &[usize]) {
    let path = format!("{outdir}/{name}");
    let mut out = String::with_capacity(v.len() * 7);
    for x in v {
        out.push_str(&x.to_string());
        out.push('\n');
    }
    if let Err(e) = std::fs::write(&path, out) {
        die(&format!("{path}: {e}"));
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(corpus), Some(size), Some(outdir)) = (args.next(), args.next(), args.next()) else {
        eprintln!("usage: seg_dump <corpus> <size-bytes> <out-dir>");
        std::process::exit(2);
    };
    let size: usize = match size.parse() {
        Ok(v) => v,
        Err(e) => die(&format!("size {size:?} is not a number: {e}")),
    };
    let text = match std::fs::read(&corpus) {
        Ok(b) => b,
        Err(e) => die(&format!("{corpus}: {e}")),
    };

    let full = seam::despaced_text(&text, usize::MAX).0.len();
    if size > full {
        die(&format!("{corpus} holds {full} despaced bytes, {size} asked for"));
    }
    let (bytes, truth) = seam::despaced_text(&text, size);

    let f = seam::analyze_with(&bytes, &SeamConfig::default());
    // Offset zero is the start of the input, not an internal boundary. Every
    // segmenter is scored on internal cuts only, so it is dropped here rather
    // than in each scorer, where one of them would forget.
    let cuts: Vec<usize> = f.cuts.iter().copied().filter(|&c| c > 0).collect();
    let r = seam::recovery(&cuts, &truth, 2);

    let despaced = format!("{outdir}/despaced.bin");
    if let Err(e) = std::fs::write(&despaced, &bytes) {
        die(&format!("{despaced}: {e}"));
    }
    write_offsets(&outdir, "truth.txt", &truth);
    write_offsets(&outdir, "trex_cuts.txt", &cuts);

    println!("corpus={corpus}");
    println!("despaced_bytes={}", bytes.len());
    println!("truth_boundaries={}", truth.len());
    println!("trex_cuts={}", cuts.len());
    println!("trex_precision={:.6}", r.precision);
    println!("trex_recall={:.6}", r.recall);
    println!("trex_f1={:.6}", r.f1);
}
