//! What the echo field's frame table costs, at sizes where narrowing it would be
//! readable.
//!
//! `EchoField` holds one `EchoFrame` a token and the frame is 44 bytes, pinned by
//! an assertion in echo.rs. Over the engine-surface corpus that is 2,650,000
//! frames, 116 MB, and `echo: a frame a token` reads 12.706 ms - about 9 GB/s, so
//! the phase is memory bandwidth and a narrower frame moves it proportionally.
//!
//! Three fields are wider than they need to be: `back_lag` and `fwd_lag` are
//! `Option<u32>` for want of a niche where a lag between two distinct
//! occurrences is never zero, and `period` is an `Option<f32>` where an f32
//! carrying NaN for absent is half the width. Together 44 becomes 32.
//!
//! That change has never been measurable, which is why it is still unmade. It is
//! worth about 3.5 ms of a 12.7 ms phase inside an 89 ms field - under the five
//! percent the engine surface can resolve - so it has been priced and parked
//! rather than made and judged on taste. This is the instrument that makes it
//! readable: the same phase at two and four times the corpus, where the same
//! proportion is tens of milliseconds rather than three.
//!
//! The phase is read out of the trace rather than timed here, so what is
//! reported is the code's own boundary and not a replica of it.
//!
//! The control is the lex, and it has to be, because every phase inside the
//! echo field touches the frames: `group_tokens` takes them mutably and writes
//! into them while it keys, so a narrower frame makes keying faster too. Reading
//! the framing against the keying is reading a change against itself, which is
//! what the first pass over this harness did - keying moved 0.755 where framing
//! moved 0.663, and neither said what the box had done. The lex runs before any
//! frame exists and no frame's width can reach it, so it is the arm that carries
//! the box.
//!
//! A reading smaller than the lex column's own movement between two runs is not
//! a reading.
//!
//! Run: `cargo run --release --example echo_frame_rate`.

use std::mem::size_of;
use std::time::Duration;

use trex::echo::EchoFrame;
use trex::token::Token;
use trex::trace;

/// Statements of four shapes, `benches/engine_surface`'s corpus exactly, so a
/// reading here and a reading there are over the same bytes.
fn corpus(statements: usize) -> String {
    let mut s = String::new();
    for i in 0..statements {
        match i % 4 {
            0 => s.push_str(&format!("let value_{i} = {} ;\n", i * 37)),
            1 => s.push_str(&format!("call_{i}(alpha, beta, {}) ;\n", i)),
            2 => s.push_str(&format!("key_{i}: item_{i}, item_{}, item_{} ;\n", i + 1, i + 2)),
            _ => s.push_str(&format!("if (cond_{i}) {{ do_{i}(x) ; }}\n")),
        }
    }
    s
}

/// One named phase's time out of one echo analysis.
///
/// Whatever the tallies already hold is dropped first, so what comes back is
/// this call's alone rather than this call's added to an earlier one.
fn phase_of(name: &str, tokens: &[Token], bytes: &[u8]) -> Duration {
    drop(trace::take_phases());
    let field = trex::echo::analyze(tokens, bytes);
    std::hint::black_box(&field);
    trace::take_phases()
        .into_iter()
        .find(|(phase, _, _)| *phase == name)
        .map(|(_, _, spent)| spent)
        .expect("the phase reported itself")
}

/// The median of three readings, in milliseconds.
fn median_ms(name: &str, tokens: &[Token], bytes: &[u8]) -> f64 {
    let mut times: Vec<f64> =
        (0..3).map(|_| phase_of(name, tokens, bytes).as_secs_f64() * 1000.0).collect();
    times.sort_by(f64::total_cmp);
    times[1]
}

/// The median of three lexes of the same bytes, in milliseconds.
///
/// The control arm. A lex builds no frame, so no frame's width reaches it, and
/// what it does between two runs is what the box did.
fn median_lex_ms(bytes: &[u8]) -> f64 {
    let mut times: Vec<f64> = (0..3)
        .map(|_| {
            let began = std::time::Instant::now();
            let tokens = trex::lexer::lex(bytes);
            let spent = began.elapsed();
            std::hint::black_box(&tokens);
            spent.as_secs_f64() * 1000.0
        })
        .collect();
    times.sort_by(f64::total_cmp);
    times[1]
}

fn main() {
    trace::record();
    let frame = size_of::<EchoFrame>();
    println!("trex {}", env!("CARGO_PKG_VERSION"));
    println!("a frame is {frame} bytes\n");
    println!(
        "{:>10}  {:>11}  {:>9}  {:>10}  {:>10}  {:>10}  {:>7}",
        "statements", "tokens", "table", "framing", "keying", "lex", "GB/s"
    );

    for statements in [200_000usize, 400_000, 800_000] {
        let bytes = corpus(statements).into_bytes();
        let tokens = trex::lexer::lex(&bytes);
        // A warm pass first: the first analysis over a fresh corpus spends
        // time mapping pages the later ones find already mapped.
        phase_of("echo: a frame a token", &tokens, &bytes);

        let framing = median_ms("echo: a frame a token", &tokens, &bytes);
        let keying = median_ms("echo: keying the tokens", &tokens, &bytes);
        let lex = median_lex_ms(&bytes);
        let table = tokens.len() * frame;
        println!(
            "{statements:>10}  {:>11}  {:>6} MB  {framing:>7.3} ms  {keying:>7.3} ms  \
             {lex:>7.3} ms  {:>7.2}",
            tokens.len(),
            table / (1024 * 1024),
            table as f64 / framing * 1e3 / 1e9
        );
    }

    println!(
        "\nthe framing column is what a narrower frame moves, the keying column moves\n\
         with it because keying writes the frames too, and the lex column is the one\n\
         that says what the box did."
    );
}
