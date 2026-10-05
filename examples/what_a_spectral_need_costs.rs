//! What each spectral reading costs on its own, and what they share.
//!
//! `the spectral field` is 479.968 ms summed over the shapes
//! `examples/where_the_time_goes` times, the largest leaf there after the
//! sweep's own attempts. It divides by need already, because a pattern spends
//! time on the readings its atoms name and no others: onset 172.348 ms, period
//! 147.917, texture 93.257, entropy 66.446, which is that 479.968 exactly.
//!
//! Those four are not four separate costs, though, and the difference decides
//! what is worth doing. `analyze_needing` is one causal forward pass with the
//! needs as guards inside the byte loop, so every reading costs the pass
//! itself - the filterbank, the bands, the walk - plus its own block. A
//! reading's own block is what could be made cheaper; the shared pass is a cost
//! every one of them would keep.
//!
//! Two readings and their pair separate the two without a clock inside the
//! loop. Where `cost(a) = base + a` and `cost(a, b) = base + a + b`, the base is
//! `cost(a) + cost(b) - cost(a, b)`. A pattern naming two spectral atoms names
//! both needs, so the pair is one scan like the others.
//!
//! What this cannot do is see a reading that shares work with one other and not
//! the rest. The identity assumes a base common to both and blocks that do not
//! overlap; two readings that share a sub-computation of their own would show up
//! as a larger base for that pair than for another, which is why every pair is
//! read rather than one.
//!
//! The figures above are over the generated statements, and the first argument
//! is a corpus path for that reason. What a reading costs is a question about
//! the bytes: the four shapes in rotation are as regular as text gets, while
//! prose carries a period at 9 offsets of 1206 and real source at 3055 of
//! 3603. A split taken over the generator prices the generator.

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

/// The four readings, spelled as the atom that names each.
const NEEDS: [(&str, &str); 4] = [
    ("onset", "\\F{onset}"),
    ("period", "\\F{period}"),
    ("texture", "\\F{texture:code}"),
    ("entropy", "\\F{entropy>0.7}"),
];

/// How many times each configuration is read. One reading is not one on this
/// box: an untouched shape moves by a seventh between two runs of the engine's
/// own surface, and the differences this harness takes are smaller than that.
const ROUNDS: usize = 5;

/// One configuration's readings, reduced.
struct Timed {
    mid: f64,
    lo: f64,
    hi: f64,
}

/// Scan `src` `ROUNDS` times, watched, and answer what `the spectral field`
/// took: the middle reading, and the two ends so a reader can see whether the
/// middle means anything.
///
/// The scan itself is dropped: the sweep and the matches are another shape's
/// question, and the field is built once whatever the sweep then does with it.
fn field_ms(src: &str, input: &[u8]) -> Timed {
    let p = trex::parse(src).unwrap_or_else(|e| panic!("{src} parses: {e:?}"));
    // An unwatched scan first, so the pages the field is written into are
    // mapped and filled before any that is timed.
    drop(trex::scan(&p, input));

    let mut got = Vec::with_capacity(ROUNDS);
    for _ in 0..ROUNDS {
        trex::trace::record();
        drop(trex::trace::take_phases());
        drop(trex::scan(&p, input));
        let phases = trex::trace::take_phases();
        trex::trace::stop();
        got.push(
            phases
                .iter()
                .find(|(n, _, _)| *n == "the spectral field")
                .map_or(0.0, |(_, _, d)| d.as_secs_f64() * 1e3),
        );
    }
    got.sort_by(f64::total_cmp);
    Timed { mid: got[got.len() / 2], lo: got[0], hi: got[got.len() - 1] }
}

fn main() {
    // A corpus by path, because what each reading costs turns on what the
    // bytes hold: the period block's work is bounded by its budget but the
    // readings it takes are not, and prose finds a period at 9 offsets of
    // 1206 where real source finds one at 3055 of 3603. With no path the
    // generated statements still run and say so, so a reading over four
    // shapes in rotation is never taken for one over text somebody wrote.
    let (input, whose) = match std::env::args().nth(1) {
        Some(path) => match std::fs::read(&path) {
            Ok(bytes) => (bytes, path),
            Err(e) => {
                eprintln!("the corpus at {path} could not be read: {e}");
                std::process::exit(2);
            }
        },
        None => (
            code(200_000).into_bytes(),
            "generated statements, four shapes in rotation".to_string(),
        ),
    };
    println!("trex {}", env!("CARGO_PKG_VERSION"));
    println!("corpus {whose}, {} bytes\n", input.len());

    let alone: Vec<Timed> = NEEDS.iter().map(|(_, src)| field_ms(src, &input)).collect();
    println!("{:>10} {:>10} {:>10} {:>10}", "need", "alone ms", "low", "high");
    for ((name, _), t) in NEEDS.iter().zip(&alone) {
        println!("{name:>10} {:>10.3} {:>10.3} {:>10.3}", t.mid, t.lo, t.hi);
    }

    println!(
        "\n{:>10} {:>10} {:>10} {:>10} {:>10} {:>8} {:>10}",
        "pair", "", "together", "sum alone", "sum - both", "sets", "expected"
    );
    // A pair whose needs nest is not two costs over a shared pass: the wider
    // reading already computes the narrower, so asking for both asks for the
    // wider alone. There `sum - both` is the narrower's own total and is a
    // check on the readings rather than a base, and only the pairs whose sets
    // are apart say anything about what the pass costs.
    let mut bases = Vec::new();
    for (i, (a_name, a_src)) in NEEDS.iter().enumerate() {
        for (j, (b_name, b_src)) in NEEDS.iter().enumerate().skip(i + 1) {
            let pair_src = format!("{a_src} {b_src}");
            let both = field_ms(&pair_src, &input);
            let sum = alone[i].mid + alone[j].mid;
            let diff = sum - both.mid;
            let (a_needs, b_needs) = (needs_of(a_src), needs_of(b_src));
            let union = needs_of(&pair_src);
            let nested = union == a_needs || union == b_needs;
            let expected = if nested {
                // The narrower reading's own total, which the wider one carries.
                alone[i].mid.min(alone[j].mid)
            } else {
                bases.push(diff);
                f64::NAN
            };
            println!(
                "{a_name:>10} {b_name:>10} {:>10.3} {sum:>10.3} {diff:>10.3} {:>8} {}",
                both.mid,
                if nested { "nest" } else { "apart" },
                if nested { format!("{expected:>10.3}") } else { format!("{:>10}", "-") }
            );
        }
    }

    // Only the pairs whose sets are apart estimate the pass they share, and
    // they estimate the same thing, so a spread among them is what the box is
    // worth on this reading rather than a division of the work.
    let lo = bases.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = bases.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    println!("\nthe {} pairs whose sets are apart read the shared pass at", bases.len());
    println!("{lo:.3} to {hi:.3} ms, and every nested pair's column should match the");
    println!("expected one beside it - where it does not, the readings are not the shape");
    println!("the need sets say they are.");
}

/// What readings a pattern's spectral atoms name.
///
/// The same answer `Analyses::build` reads to decide what to compute, so a pair
/// is nested here exactly where the engine builds one field for both.
fn needs_of(src: &str) -> trex::spectral::Needs {
    trex::parse(src).unwrap_or_else(|e| panic!("{src} parses: {e:?}")).spectral_needs()
}
