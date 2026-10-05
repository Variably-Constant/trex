//! Reading the property axes as a library. Each axis is a plain function
//! returning a field keyed by byte offset; this walks five of them.
//!
//! Run: `cargo run --release --example axes`

use trex::{OrbitGroup, magnitude, orbit, seam, shape, stress};

fn main() {
    // Magnitude: the order of magnitude of each token's value. The huge
    // config value is a scale outlier no other axis sees.
    let mag = magnitude::analyze_bytes(b"retries = 3 ; max_bytes = 5000000000");
    let peak = mag.frames.iter().map(|f| f.magnitude).fold(0.0f32, f32::max);
    println!("magnitude: {} tokens, peak {peak:.2}, total energy {:.1}", mag.n_tokens, mag.total_energy);

    // Stress: bracket-nesting load. A deeply nested call builds depth.
    let st = stress::analyze_bytes(b"f(g(h(x)))");
    println!("stress:    max depth {}, {} peak(s)", st.max_depth, st.peaks.len());

    // Shape: the silhouette period of a ragged table, width-free.
    let sh = shape::analyze_bytes(b"1,22,3\n444,5,66\n7,888,9");
    let period = sh.frames.iter().map(|f| f.period).max().unwrap_or(0);
    println!("shape:     {} tokens, dominant period {period}", sh.n_tokens);

    // Seam: bidirectional predictive segmentation, no dictionary.
    let sm = seam::analyze(b"the cat sat");
    println!("seam:      {} segments", sm.segments().len());

    // Orbit: fold symmetry-equivalent tokens to one representative.
    let (raw, orbits) = orbit::collapse_stats(b"cat dog bat the", OrbitGroup::Shape);
    println!("orbit:     {raw} raw forms -> {orbits} shape orbits");
}
