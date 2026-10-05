//! Using trex as a library: parse a pattern once, scan bytes, read matches
//! and their captured registers, rewrite with a template, and read a
//! property axis - the whole surface most callers need.
//!
//! Run: `cargo run --release --example scan_basics`

use trex::{Template, captures, magnitude, parse, rewrite, scan};

fn main() {
    // 1. Parse a pattern once; scan as many inputs as you like against it.
    //    `<\W:t>.*</=t>` binds the open-tag word to `t` and requires the
    //    close tag to equal it - a balanced, back-referenced match. A scan
    //    reports byte spans; `captures` resolves the registers each bound.
    let pat = parse(r"<\W:t>.*</=t>").expect("valid pattern");
    let input = b"<div>hi</div> <span>yo</span>";
    let spans = scan(&pat, input);
    println!("matches:");
    for m in captures(&pat, input, &spans) {
        let text = std::str::from_utf8(&input[m.start..m.end]).unwrap();
        // captures is a Vec<(name, span)> sorted by name; `group` slices the
        // input at the span the register bound.
        let tag = m.group("t", input).map_or("", |b| std::str::from_utf8(b).unwrap_or(""));
        println!("  [{}..{}] {text:?}  tag={tag:?}", m.start, m.end);
    }

    // 2. Rewrite: replace each match with a rendered template. The template
    //    is validated against the pattern's capture names.
    let epat = parse(r"\E:e").expect("valid pattern");
    let tmpl = Template::parse("<${e:upper}>", &["e".to_string()]).expect("valid template");
    let out = rewrite(&epat, &tmpl, b"reach me at bob@x.com today");
    println!("rewrite: {}", String::from_utf8_lossy(&out));

    // 3. A property axis is a plain library call returning a field. The huge
    //    config value is the scale outlier the axis exists to catch.
    let field = magnitude::analyze_bytes(b"retries = 3 ; max_bytes = 5000000000");
    let peak = field.frames.iter().map(|f| f.magnitude).fold(0.0f32, f32::max);
    println!("magnitude: {} tokens, peak {peak:.2}, total energy {:.1}", field.n_tokens, field.total_energy);
}
