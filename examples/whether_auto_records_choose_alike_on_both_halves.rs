//! Whether `--record auto` makes the same choice on both halves of a real
//! file, and what the choice gains over always cutting at the seam or always
//! at the weakest bonds.
//!
//! `auto` takes whichever of `seam` and `bind` puts more of its cuts at the
//! input's line starts, a boundary neither is built from, the two making as
//! many cuts. A rule chosen per input is only a property of the input where it
//! holds across it: if the two halves of one file pick differently, the choice
//! follows noise in where the cuts are rather than anything about the file. So
//! each file is cut at the line boundary nearest its middle and each half
//! chooses alone.
//!
//! The share reported is the fraction of a unit's cuts that land on a line
//! start, the quantity the choice is made on.
//!
//! Each file is read as `trex scan` reads it (`trex::encoding::decode`).
//!
//! Run: `cargo run --release --example whether_auto_records_choose_alike_on_both_halves -- <file>...`

use trex::records::RecordUnit;

/// The cut offsets a unit makes: every record's start but the input's first.
fn cuts(unit: &RecordUnit, input: &[u8]) -> Vec<usize> {
    unit.records(input).iter().map(|&(s, _)| s).filter(|&s| s > 0).collect()
}

/// The share of `cuts` that land on a line start, the byte after a newline.
fn at_line_starts(cuts: &[usize], input: &[u8]) -> f64 {
    if cuts.is_empty() {
        return 0.0;
    }
    cuts.iter().filter(|&&c| input[c - 1] == b'\n').count() as f64 / cuts.len() as f64
}

/// The two shares on `input`, and which `auto` takes.
fn choose(input: &[u8]) -> (f64, f64, &'static str) {
    let seam = at_line_starts(&cuts(&RecordUnit::Seam, input), input);
    let bind = at_line_starts(&cuts(&RecordUnit::Bind(None), input), input);
    let auto = cuts(&RecordUnit::Auto, input);
    let chose = if auto == cuts(&RecordUnit::Seam, input) { "seam" } else { "bind" };
    (seam, bind, chose)
}

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: whether_auto_records_choose_alike_on_both_halves <file>...");
        std::process::exit(2);
    }
    println!(
        "{:<34} {:>11} {:>11} {:>6}   {:>11} {:>11} {:>6}   agree",
        "file", "seam first", "bind first", "takes", "seam second", "bind second", "takes"
    );
    let (mut files, mut agree, mut unreadable) = (0usize, 0usize, 0usize);
    for path in &paths {
        let raw = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                println!("{path}: unreadable: {e}");
                unreadable += 1;
                continue;
            }
        };
        let bytes = trex::encoding::decode(raw);
        let mid = bytes.len() / 2;
        let cut = match bytes[mid..].iter().position(|&b| b == b'\n') {
            Some(i) => mid + i + 1,
            None => mid,
        };
        let (a, b) = (&bytes[..cut], &bytes[cut..]);
        let (sa, ba, ca) = choose(a);
        let (sb, bb, cb) = choose(b);
        files += 1;
        if ca == cb {
            agree += 1;
        }
        let name = std::path::Path::new(path).file_name().map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
        println!(
            "{name:<34} {sa:>11.3} {ba:>11.3} {ca:>6}   {sb:>11.3} {bb:>11.3} {cb:>6}   {}",
            if ca == cb { "yes" } else { "NO" }
        );
    }
    println!("\n{agree} of {files} files choose alike on both halves");
    if unreadable > 0 {
        std::process::exit(1);
    }
}
