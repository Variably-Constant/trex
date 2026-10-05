//! Which rung answers each operation, one call apiece.
//!
//! `TREX_TRACE=1 cargo run --release --example route_trace` prints one line per
//! call naming the rung that answered it. The comparison in
//! `benches/vs_regex_full.rs` calls each operation thousands of times, so
//! tracing it prints a line per call and is unreadable. This runs each one a
//! single time, over a corpus of the same shape and size, so the output is one
//! screen and every line is attributable.
//!
//! It exists because a rung read off a millisecond count has been wrong: a row
//! measured at sixteen milliseconds and a row measured at three can enter the
//! same function and leave by different rungs, and "captures at every match"
//! was once attributed to the resumable walk when it never touches one.
//!
//! Without `TREX_TRACE` set it prints nothing but the answers, which is what
//! makes it safe to run under `cargo test` as a compile-and-run check.

fn corpus() -> Vec<u8> {
    // The comparison's shape: tag lines with a binding, a number and a quoted
    // string, so every pattern below finds matches spread through the input
    // rather than clustered at one end.
    let mut text = String::with_capacity(7_400_000);
    let mut i = 0u32;
    while text.len() < 7_340_000 {
        text.push_str(&format!(
            "let alpha_{} = {} ; cond_{} \"q{}\" beta = {} ;\n",
            i % 97,
            i % 1000,
            i,
            i % 13,
            i % 7
        ));
        i += 1;
    }
    text.into_bytes()
}

fn main() {
    let input = corpus();
    println!("corpus {} bytes, TREX_TRACE {}", input.len(), if trex::trace::on() { "on" } else { "off" });

    for src in [
        "\"alpha_1\"",
        "\\W",
        "\\N",
        "\\Q",
        "\\W \"=\"",
        "`cond_[0-9]+`",
        "\"let\" \\W \"=\"",
        // The binding twin of the line above, which the comparison pairs it
        // with. The two take different rungs wherever a route is written
        // against a shape the language spells without bindings, and a trace
        // holding only one of them cannot show that.
        "\"let\" \\W:v \"=\"",
        "\\W:name \"=\"",
        "\\W{2}",
        "^ \"let\"",
        // The shapes that reach an engine rather than a route, which is where
        // the time now is: a balanced group at 44.6203 ms over 7.34 MB, a star
        // at 37.3523, an alternation at 14.0998 and an unequal repeat at
        // 11.3008. Three different engines answer those four, and a trace
        // holding none of them cannot say which.
        "\\B",
        "\"let\" \\W* \"=\"",
        "(\\N | \\W) \"=\"",
        "\\W{2,4}",
    ] {
        let p = trex::parse(src).expect("pattern parses");
        println!("\n--- {src} ---");

        // The offset every anchored operation is asked from is the end of the
        // first match, which is what a caller walking an input would use.
        let at = trex::find(&p, &input).map_or(0, |s| s.end());

        let spans = trex::scan(&p, &input);
        println!("  scan                {} matches", spans.len());
        println!("  is_match            {}", trex::is_match(&p, &input));
        println!("  find                {:?}", trex::find(&p, &input).map(|s| s.start()));
        println!("  find_at({at})       {:?}", trex::find_at(&p, &input, at).map(|s| s.start()));
        println!("  shortest_match      {:?}", trex::shortest_match(&p, &input));
        println!("  shortest_match_at   {:?}", trex::shortest_match_at(&p, &input, at));
        println!("  captures            {} resolved", trex::captures(&p, &input, &spans).len());
        println!("  captures_at({at})   {:?}", trex::captures_at(&p, &input, at).is_some());
        // The two cursor forms, which take their own ladders and were the
        // largest remaining block when this line was added: without them a
        // trace of this file says nothing about the rungs they take.
        println!("  captures_iter       {} matches", trex::captures_iter(&p, &input).count());
        println!(
            "  captures_read_iter  {}",
            match trex::captures_read_iter(&p, &input) {
                Some(mut c) => {
                    let mut slots = trex::CaptureSlots::of(&p);
                    let mut n = 0usize;
                    while c.next_into(&mut slots).is_some() {
                        n += 1;
                    }
                    format!("{n} matches")
                }
                None => "declined".to_string(),
            }
        );
        println!("  splitn(4)           {} pieces", trex::splitn(&p, &input, 4).count());
    }
}
