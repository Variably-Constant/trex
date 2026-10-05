//! What a byte-grain pass over the corpus costs, against what the sweep spends
//! entering attempts it then kills at once.
//!
//! The sweep's cost is entering an attempt rather than succeeding at one. A
//! pattern where 1,538,617 of 1,650,000 attempts survive is 17.7 ns an attempt
//! and one where four survive is 16.3, so the per-attempt cost is flat in what
//! the attempts find and an attempt not made is its cost saved. Eighty-six per
//! cent of them die on the way in, which is about 27 ms a scan that a prefilter
//! over the anchors could reach for.
//!
//! This is the other half of that subtraction: what such a prefilter would cost.
//! The automaton is the one `byte_lex` builds and `byte_dfa` determinizes as it
//! walks, over `benches/engine_surface`'s corpus - the same bytes the sweep
//! figure is over, so the two numbers are about one workload.
//!
//! Three things vary here, because a single reading of one automaton over one
//! corpus answers less than it appears to.
//!
//! The corpus, because a lazy table is built by what walks it: code of four
//! statement shapes reaches few states, and a corpus carrying digests, uuids,
//! addresses, colors and base64 reaches the ones those recognizers need. A rate
//! that held only on the narrow corpus would be an artifact of the corpus.
//!
//! The automaton, because a prefilter for a pattern carries the recognizers that
//! pattern's first atoms can begin with and not the whole gate. The number and
//! the word alone stand for that end of the range.
//!
//! And cold against warm, because a prefilter meets both: the table is built as
//! it is walked, so a first pass spends time building the states it reaches and
//! later passes over similar bytes do not.
//!
//! Run: `cargo run --release --example byte_grain_rate`.

use std::time::Instant;

use trex::byte_dfa::ByteDfa;
use trex::byte_nfa::{Builder, ByteNfa};

/// Statements of four shapes, `benches/engine_surface`'s corpus exactly, so a
/// reading here and a reading there are over the same bytes.
fn code(statements: usize) -> String {
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

/// A line of every kind the gate reads, so the walk reaches the states the
/// narrow corpus never asks for.
fn varied(records: usize) -> String {
    let mut s = String::new();
    for i in 0..records {
        let o = i % 256;
        s.push_str(&format!(
            "rec_{i} sum=a{i:031x} id={i:08x}-{i:04x}-{i:04x}-{i:04x}-{i:012x} \
             mac={o:02x}:{o:02x}:{o:02x}:{o:02x}:{o:02x}:{o:02x} tint=#{:06x} \
             blob=aB{i:012x}Yz size={}KB wait={}ms share={}.{}% n={} \
             url=http://h{i}.example/p/{i} jwt=eyJh{i:04x}.eyJz{i:04x}.Sfl-{i:04x} \
             w=word_{i} ;\n",
            i * 1234,
            i * 3,
            i * 7,
            i % 100,
            i,
            i * 37
        ));
    }
    s
}

/// The number and the word alone: the small end of what a prefilter carries.
fn narrow() -> ByteNfa {
    let mut b = Builder::new();
    let num = b.adopt(&trex::byte_lex::number());
    let wrd = b.adopt(&trex::byte_lex::word());
    let both = b.or(num, wrd);
    b.build(both)
}

/// Read the whole corpus as the gate divides it: how many runs it cut, and how
/// many bytes those runs covered.
///
/// A position the gate claims nothing at advances by one, which is what a
/// prefilter would do with a byte no recognizer can begin on.
fn walk(dfa: &mut ByteDfa, input: &[u8]) -> (usize, usize) {
    let (mut cut, mut covered) = (0usize, 0usize);
    let mut at = 0usize;
    while at < input.len() {
        match dfa.recognize(&input[at..]) {
            Some((len, _)) if len > 0 => {
                cut += 1;
                covered += len;
                at += len;
            }
            _ => at += 1,
        }
    }
    (cut, covered)
}

/// The median of `times`, and the fastest and slowest, so a reading carries the
/// spread it was taken over rather than one number.
fn median(mut times: Vec<f64>) -> (f64, f64, f64) {
    times.sort_by(f64::total_cmp);
    let mid = times[times.len() / 2];
    (mid, times[0], times[times.len() - 1])
}

/// One reading: an automaton over a corpus, cold then warm.
fn read(label: &str, nfa: &ByteNfa, input: &[u8]) {
    let mut cold = ByteDfa::new(nfa);
    let started = Instant::now();
    let (cut, covered) = walk(&mut cold, input);
    let cold_ms = started.elapsed().as_secs_f64() * 1000.0;
    let cold_states = cold.built();

    let mut warm = ByteDfa::new(nfa);
    walk(&mut warm, input);
    let mut times = Vec::new();
    for _ in 0..5 {
        let started = Instant::now();
        let seen = walk(&mut warm, input);
        times.push(started.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(seen, (cut, covered), "a warm pass read the corpus differently");
    }
    let (mid, low, high) = median(times);
    let bytes = input.len() as f64;
    println!(
        "{label:<34} {:>6} states   cold {cold_ms:8.3} ms {:5.2} ns/B ({cold_states} built)   \
         warm {mid:8.3} ms {:5.2} ns/B ({} built) [{low:.3}, {high:.3}]   \
         {cut} runs, {:.1}% covered",
        nfa.len(),
        cold_ms * 1e6 / bytes,
        mid * 1e6 / bytes,
        warm.built(),
        100.0 * covered as f64 / bytes
    );
}

fn main() {
    let engine_surface = code(200_000).into_bytes();
    let mixed = varied(30_000).into_bytes();
    let gate = trex::byte_lex::tokens();
    let small = narrow();

    println!("trex {}", env!("CARGO_PKG_VERSION"));
    println!("engine_surface {} bytes, varied {} bytes\n", engine_surface.len(), mixed.len());

    read("every recognizer, engine_surface", &gate, &engine_surface);
    read("every recognizer, varied", &gate, &mixed);
    read("number and word, engine_surface", &small, &engine_surface);
    read("number and word, varied", &small, &mixed);

    println!(
        "\nthe sweep enters 1,650,000 attempts a scan over engine_surface, at 16.3 ns an\n\
         attempt and up, so 26.9 ms a scan is the floor a prefilter would be spending\n\
         this to save, and 86% of those attempts die on the way in."
    );
}
