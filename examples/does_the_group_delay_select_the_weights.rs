//! Whether the group delay chooses the mixer's weights better than the byte
//! before does, read on real files in bits per byte.
//!
//! The group delay is a regime reading: how long ago the stream's current
//! rhythm was laid down, and whether the stream is in steady rhythm or the wake
//! of a transient. As a predictor of its own in the mixer it earned a place
//! only where it is large enough to clear the overhead of an extra input -
//! `does_the_group_delay_help_the_coder`. But a regime reading's natural place
//! is not a prediction. The mixer keeps one weight set per selector value so it
//! can weight its models differently in different regimes, and the shipped
//! selector is the byte before - a regime read over one byte. This asks whether
//! a regime read over the bank's memory chooses those weights better.
//!
//! The arms, each through `seam::logistic_mix_bits_with_selector`:
//!
//!   plain      the shipped selector, the byte before, 256 sets
//!   nib        the high nibble of the byte before, 16 sets - coarser, and the
//!              control the nibble arms below differ from
//!   nib x gd   that nibble crossed with the group delay's bin, 128 sets
//!   nib x gd~  the same with the group delay moved half the stream along
//!   nib x pw   the nibble crossed with the power's bin, whether the power is
//!              the better regime of the two
//!   nib x pw~  the same with the power moved half the stream along
//!   byte x gd  the whole byte before crossed with the group delay, 2048 sets
//!   byte x gd~ the same, shifted
//!   nib x pw x gd   the nibble crossed with both readings, 1024 sets: whether
//!                   the group delay still chooses once the power does
//!   nib x pw x gd~  the same with the group delay alone shifted
//!
//! and three through `seam::logistic_mix_bits_with_second_selector`, where the
//! shipped mixer keeps the byte before and a second mixer beside it takes the
//! selector, a final mixer learning how far to trust each - the placement that
//! does not have to know which kind of stream it is coding:
//!
//!   beside npg   a second mixer chosen by the nibble, the power and the group
//!                delay, 1024 sets
//!   beside npg~  the same with the group delay alone shifted
//!   beside np    a second mixer chosen by the nibble and the power alone
//!   beside nib   a second mixer chosen by the nibble alone, 16 sets: the
//!                selector with no rhythm in it
//!   beside one   a second mixer with one weight set, chosen by nothing: the
//!                control, since a second mixer and the final mixer over it earn
//!                something from their weights whatever chooses them, and a
//!                selector's gain is its gain over this
//!
//! and `as shipped`, the second mixer `trex compress --second-mixer` codes
//! with (`seam::second_mixer_bits_with`), its side information counted. It
//! reads the rhythm at `seam::RHYTHM_R` and the loudest band's age in the
//! shape `seam::RhythmShape::SHIPPED`, cuts the power at bin edges a decoder is
//! sent where `beside npg` cuts it by rank, and reads the bank at the last
//! whole line where `beside npg` reads it at each token's end.
//! `one + shipped` is three mixers through
//! `seam::logistic_mix_bits_with_mixers_beside`: the shipped one, the one with
//! one weight set, and the rhythm's as shipped, the final mixer over all three.
//!
//! A selector is read before the byte it codes: the group delay and the power
//! are the bank's state after the last token ending strictly before the byte,
//! the rule the mixer's shape input follows. The shifted arms keep every value
//! and none of its position, so a gain over one is a gain from the regime and
//! not from having more weight sets.
//!
//! The mixer runs with its orbit models on and no warm seed.
//!
//! `--r` sets the bank's pole radius, whose memory is about `r / (1 - r)`
//! tokens, and `--age dom` reads the loudest band's own group delay in place of
//! the power-weighted mean. `--delay D` holds every reading back until `D`
//! bytes past the token's end: a recognizer can read past the token it emits
//! (`does_the_lexer_read_ahead`), so a gain that survives the delay is not one
//! carried by that reach. `--prior` starts every arm's mixer from the baked
//! prior `trex compress` codes from, which the arms otherwise run without.
//! `--time R` prints no table of arms: it times the shipped mixer against the
//! second mixer beside it chosen by the nibble, the power and the group delay,
//! alternately over `R` rounds in one process, and prints the selectors' own
//! cost apart, which is what that placement would cost in production.
//! `--shipped` prints the codings `trex compress` offers and the controls its
//! second mixer is read against, and nothing else: `plain`, `as shipped`,
//! `beside one` and `one + shipped`.
//! `--grid` prints no table: it codes with the second mixer beside the shipped
//! one under every selector shape `seam::RhythmShape` names over the byte
//! before's top 0 to 8 bits, crossed with 1, 2, 4, 8 and 16 power bins and 1,
//! 2, 4, 8 and 16 group-delay bins - 225 shapes, the shipped one and the one
//! weight set among them - each read as `trex compress --second-mixer` would
//! code it, side information counted, one `GRID` line a file and shape, the
//! shapes coded across the machine's cores. It reads the bank at the shipped
//! selector's radius and the loudest band's group delay unless `--r` or
//! `--age` is given, and `--shapes 4/8/8,0/8/1` codes only the shapes listed,
//! each as byte bits, power bins and group-delay bins.
//!
//! Every input is a real file somebody produced for another purpose, read as
//! `trex compress` reads it (`trex::encoding::decode`): a UTF-16 file is coded
//! as the UTF-8 text it holds, and a UTF-8 byte order mark is dropped. Coded as
//! its raw bytes, a UTF-16 log is a stream with every other byte zero, which no
//! user of the coder hands it.
//!
//! Run: `cargo run --release --example does_the_group_delay_select_the_weights -- [--r <pole radius>] [--age mean|dom] [--delay <bytes>] [--prior] [--time <rounds>] [--shipped] [--grid [--shapes <list>]] <file>...`

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use trex::resonator::{Bank, DEFAULT_R, default_periods};
use trex::seam::{
    PreloadTable, RhythmAge, RhythmReadings, RhythmShape, logistic_mix_bits_with, logistic_mix_bits_with_mixers_beside,
    logistic_mix_bits_with_second_selector, logistic_mix_bits_with_selector,
};

/// The bin counts `--grid` cuts each rhythm reading into.
const GRID_BINS: [u16; 5] = [1, 2, 4, 8, 16];

/// Every shape `--grid` codes with: the byte before's top 0 to 8 bits crossed
/// with every power and group-delay bin count in `GRID_BINS`.
fn grid_shapes() -> Vec<RhythmShape> {
    let mut shapes = Vec::new();
    for byte_bits in 0..=8u8 {
        for power_bins in GRID_BINS {
            for age_bins in GRID_BINS {
                shapes.push(RhythmShape { byte_bits, power_bins, age_bins });
            }
        }
    }
    shapes
}

/// `work` done for every job from `0..jobs` across the machine's cores, each
/// result in its job's place. The jobs are handed out one at a time from a
/// shared counter, so a slow job holds up no other.
fn in_parallel<T: Send, F: Fn(usize) -> T + Sync>(jobs: usize, work: F) -> Vec<T> {
    let cores = std::thread::available_parallelism().expect("the machine reports its cores").get();
    let next = AtomicUsize::new(0);
    let mut placed: Vec<Option<T>> = (0..jobs).map(|_| None).collect();
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..cores.min(jobs).max(1))
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let job = next.fetch_add(1, Ordering::Relaxed);
                        if job >= jobs {
                            break;
                        }
                        done.push((job, work(job)));
                    }
                    done
                })
            })
            .collect();
        for worker in workers {
            for (job, result) in worker.join().expect("a worker finishes its jobs") {
                placed[job] = Some(result);
            }
        }
    });
    placed.into_iter().map(|result| result.expect("every job ran")).collect()
}

/// Bins a regime reading is cut into.
const BINS: u16 = 8;

/// `v` moved half its length along, which keeps every value and none of its
/// position.
fn shifted<T: Copy>(v: &[T]) -> Vec<T> {
    let n = v.len();
    let half = n / 2;
    (0..n).map(|i| v[(i + half) % n]).collect()
}

/// Each position's quantile bin in `score`.
fn quantile_bins(score: &[f32]) -> Vec<u16> {
    let n = score.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_unstable_by(|&a, &b| score[a].total_cmp(&score[b]));
    let mut out = vec![0u16; n];
    for (r, &i) in order.iter().enumerate() {
        out[i] = ((r * usize::from(BINS)) / n.max(1)).min(usize::from(BINS) - 1) as u16;
    }
    out
}

/// Which age a regime reading takes from the bank.
#[derive(Clone, Copy)]
enum Age {
    /// The power-weighted mean group delay: every rhythm's age, each weighted
    /// by how much rhythm it carries.
    Mean,
    /// The loudest band's own group delay: the age of the one rhythm carrying
    /// the most, unblended with the weaker ones.
    Dominant,
}

impl Age {
    fn name(self) -> &'static str {
        match self {
            Age::Mean => "the power-weighted mean",
            Age::Dominant => "the loudest band's own",
        }
    }
}

/// What the command line asks for.
struct Options {
    /// The bank's pole radius.
    r: f32,
    /// The age the group delay is read for.
    age: Age,
    /// The bytes past a token's end its reading is held back by.
    delay: usize,
    /// Whether every mixer starts from the baked prior.
    prior: bool,
    /// Rounds of the timing pass, which times the shipped mixer against the
    /// second mixer beside it in place of the table of arms.
    time: Option<usize>,
    /// Whether to print the shipped coder and the second mixer `trex compress`
    /// codes with, and no other arm.
    shipped: bool,
    /// Whether to code with the second mixer under every shape of selector.
    grid: bool,
    /// Whether `--r` and `--age` were given, which `--grid` reads the bank at
    /// in place of the shipped selector's radius and age.
    r_given: bool,
    age_given: bool,
    /// The shapes `--grid` codes with in place of every one, from `--shapes`.
    shapes: Option<Vec<RhythmShape>>,
}

/// The shapes `--shapes` names: byte bits, power bins and group-delay bins
/// joined by `/`, one shape per comma, as in `4/8/8,0/8/1`.
fn parse_shapes(list: &str) -> Result<Vec<RhythmShape>, String> {
    list.split(',')
        .map(|one| {
            let parts: Vec<&str> = one.split('/').collect();
            let [bits, power, age] = parts.as_slice() else {
                return Err(format!("{one:?} is not bits/power/age"));
            };
            let number = |s: &str| s.parse::<u16>().map_err(|e| format!("{s:?} in {one:?}: {e}"));
            let byte_bits = u8::try_from(number(bits)?).map_err(|e| format!("{bits:?} in {one:?}: {e}"))?;
            let shape = RhythmShape { byte_bits, power_bins: number(power)?, age_bins: number(age)? };
            if byte_bits > 8 || shape.power_bins == 0 || shape.age_bins == 0 || shape.sets() > 65_536 {
                return Err(format!("{one:?} is not a shape a selector can take"));
            }
            Ok(shape)
        })
        .collect()
}

/// The options from `--r <radius>`, `--age mean|dom`, `--delay <bytes>`,
/// `--prior`, `--time <rounds>` and `--shipped` at the front of `args`, which
/// are taken off it, leaving the files. A value that does not parse is refused
/// rather than defaulted, since a table labeled with one setting and read at
/// another is the failure this would otherwise hide.
fn options(args: &mut Vec<String>) -> Options {
    let (mut r, mut age, mut delay, mut prior, mut time) = (DEFAULT_R, Age::Mean, 0usize, false, None);
    let (mut shipped, mut grid, mut r_given, mut age_given, mut shapes) = (false, false, false, false, None);
    loop {
        match args.first().map(String::as_str) {
            Some("--shipped") => {
                shipped = true;
                args.drain(..1);
            }
            Some("--grid") => {
                grid = true;
                args.drain(..1);
            }
            Some("--shapes") => {
                let Some(list) = args.get(1) else {
                    eprintln!("--shapes needs a list, for example --shapes 4/8/8,0/8/1");
                    std::process::exit(2);
                };
                shapes = match parse_shapes(list) {
                    Ok(s) => Some(s),
                    Err(e) => {
                        eprintln!("--shapes: {e}");
                        std::process::exit(2);
                    }
                };
                args.drain(..2);
            }
            Some("--r") => {
                let Some(value) = args.get(1) else {
                    eprintln!("--r needs a pole radius, for example --r 0.95");
                    std::process::exit(2);
                };
                r = match value.parse::<f32>() {
                    Ok(v) if v > 0.0 && v < 1.0 => v,
                    Ok(v) => {
                        eprintln!("a pole radius is between 0 and 1, got {v}");
                        std::process::exit(2);
                    }
                    Err(e) => {
                        eprintln!("--r {value} is not a number: {e}");
                        std::process::exit(2);
                    }
                };
                r_given = true;
                args.drain(..2);
            }
            Some("--age") => {
                age = match args.get(1).map(String::as_str) {
                    Some("mean") => Age::Mean,
                    Some("dom") => Age::Dominant,
                    other => {
                        eprintln!("--age takes mean or dom, got {other:?}");
                        std::process::exit(2);
                    }
                };
                age_given = true;
                args.drain(..2);
            }
            Some("--delay") => {
                delay = match args.get(1).map(|v| v.parse::<usize>()) {
                    Some(Ok(v)) => v,
                    Some(Err(e)) => {
                        eprintln!("--delay takes a byte count: {e}");
                        std::process::exit(2);
                    }
                    None => {
                        eprintln!("--delay needs a byte count, for example --delay 64");
                        std::process::exit(2);
                    }
                };
                args.drain(..2);
            }
            Some("--prior") => {
                prior = true;
                args.drain(..1);
            }
            Some("--time") => {
                time = match args.get(1).map(|v| v.parse::<usize>()) {
                    Some(Ok(v)) if v > 0 => Some(v),
                    Some(Ok(_)) => {
                        eprintln!("--time takes a count of rounds above zero");
                        std::process::exit(2);
                    }
                    Some(Err(e)) => {
                        eprintln!("--time takes a count of rounds: {e}");
                        std::process::exit(2);
                    }
                    None => {
                        eprintln!("--time needs a count of rounds, for example --time 5");
                        std::process::exit(2);
                    }
                };
                args.drain(..2);
            }
            _ => return Options { r, age, delay, prior, time, shipped, grid, r_given, age_given, shapes },
        }
    }
}

/// Per significant token, the group delay's fixed bin and the log power, the
/// bank read after the token goes in; and the tokens, for their ends.
fn token_regimes(
    input: &[u8],
    periods: &[u16],
    r: f32,
    which: Age,
) -> (Vec<trex::token::Token>, Vec<u16>, Vec<f32>) {
    let toks: Vec<trex::token::Token> =
        trex::lexer::lex(input).into_iter().filter(|t| t.is_significant()).collect();
    let raw: Vec<u32> = toks.iter().map(|t| t.kind.code()).collect();
    let mut distinct = raw.clone();
    distinct.sort_unstable();
    distinct.dedup();
    let span = r / (1.0 - r);
    let mut bank = Bank::new(distinct.len(), periods, r);
    let mut gd = vec![0.0f32; periods.len()];
    let mut pw = vec![0.0f32; periods.len()];
    let mut ages = Vec::with_capacity(raw.len());
    let mut powers = Vec::with_capacity(raw.len());
    for s in &raw {
        bank.push(distinct.partition_point(|d| d < s) as u32);
        bank.read_into(&mut gd, &mut pw);
        let total: f32 = pw.iter().sum();
        let age = match which {
            Age::Mean if total > f32::EPSILON => {
                gd.iter().zip(&pw).map(|(g, p)| g * p).sum::<f32>() / total
            }
            Age::Mean => 0.0,
            Age::Dominant => {
                pw.iter().zip(&gd).max_by(|a, b| a.0.total_cmp(b.0)).map_or(0.0, |(_, &g)| g)
            }
        };
        ages.push(((age / span * f32::from(BINS)) as u16).min(BINS - 1));
        powers.push((total + f32::EPSILON).ln());
    }
    (toks, ages, powers)
}

/// A value a byte from the last significant token that ended more than
/// `delay` bytes before it, or `none` where no token has.
fn per_byte(len: usize, toks: &[trex::token::Token], of: &[u16], none: u16, delay: usize) -> Vec<u16> {
    let mut out = vec![none; len];
    let mut done = 0usize;
    for (t, slot) in out.iter_mut().enumerate() {
        while done < toks.len() && toks[done].end() + delay < t {
            done += 1;
        }
        if done > 0 {
            *slot = of[done - 1];
        }
    }
    out
}

/// The header of the table of arms, one column an arm.
fn print_arm_header() {
    println!(
        "{:<22} {:>10}  {:>7}  {:>7}  {:>7} {:>7}  {:>7} {:>7}  {:>8} {:>8}  {:>9} {:>8}  {:>11} {:>8}  {:>10}  \
         {:>10}  {:>10}  {:>10}  {:>13}",
        "input",
        "bytes",
        "plain",
        "nib",
        "nib*gd",
        "~",
        "nib*pw",
        "~",
        "byte*gd",
        "~",
        "nib*pw*gd",
        "~",
        "beside npg",
        "~",
        "beside np",
        "beside nib",
        "beside one",
        "as shipped",
        "one + shipped"
    );
}

fn short(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let Options { r, age: which, delay, prior: with_prior, time, shipped, grid, r_given, age_given, shapes } =
        options(&mut args);
    // The grid reads the bank as the shipped selector does unless told
    // otherwise, where the other arms read it at the harness's own defaults.
    let grid_r = if r_given { r } else { trex::seam::RHYTHM_R };
    let grid_age = match (age_given, which) {
        (true, Age::Mean) => RhythmAge::PowerWeighted,
        (false, _) | (true, Age::Dominant) => RhythmAge::Loudest,
    };
    let coded_shapes = shapes.unwrap_or_else(grid_shapes);
    if args.is_empty() {
        eprintln!(
            "usage: does_the_group_delay_select_the_weights [--r <pole radius>] [--age mean|dom] [--delay <bytes>] [--prior] [--time <rounds>] [--shipped] [--grid] <file>..."
        );
        std::process::exit(2);
    }
    // The prior `trex compress` codes from, where it asks for one.
    let prior: Option<&[PreloadTable]> = with_prior.then(trex::seam::baked_model);
    let periods = default_periods(32);

    println!("bits per byte, the production mixer with its weight set chosen by each selector.");
    println!("`~` is the regime moved half the stream along.");
    println!(
        "the bank at r {r}, a memory of about {:.0} tokens; gd is {} group delay; read {delay} bytes or more past a token's end.",
        r / (1.0 - r),
        which.name()
    );
    println!(
        "the mixer starts from {}.",
        if with_prior { "the baked prior, as trex compress does" } else { "nothing" }
    );
    println!();
    if grid {
        println!(
            "grid: {} shapes, the bank at r {grid_r} and the {grid_age:?} group delay",
            coded_shapes.len()
        );
        println!(
            "GRID lines: input, the byte before's top bits, power bins, group-delay bins, weight sets, side bits, \
             bits per byte, the default coder's bits per byte on the same input, the radius, and the age read"
        );
    } else if shipped {
        println!(
            "{:<22} {:>10}  {:>7}  {:>10}  {:>10}  {:>13}",
            "input", "bytes", "plain", "as shipped", "beside one", "one + shipped"
        );
    } else if let Some(rounds) = time {
        println!(
            "timing: the shipped mixer against the second mixer beside it chosen by nib*pw*gd, the median of \
             {rounds} rounds each, alternating, with the fastest and slowest"
        );
    } else {
        print_arm_header();
    }

    // An input that cannot be read is a row missing from the table, and a
    // table missing a row reads as complete. So a run with one exits failing.
    let mut unreadable = 0usize;
    for (n, arg) in args.iter().enumerate() {
        let path = Path::new(arg);
        let name = short(path);
        eprintln!("[{}/{}] {name}", n + 1, args.len());
        let raw = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                println!("{name:<22} unreadable: {e}");
                unreadable += 1;
                continue;
            }
        };
        let read = raw.len();
        let bytes = trex::encoding::decode(raw);
        let len = bytes.len();
        if len != read {
            eprintln!("  {name}: {read} bytes read, {len} as trex compress decodes them");
        }
        if grid {
            let t0 = Instant::now();
            let plain = logistic_mix_bits_with(&bytes, true, &[], prior) / len.max(1) as f64;
            let readings = RhythmReadings::read_with(&bytes, grid_r, grid_age);
            let coded = in_parallel(coded_shapes.len(), |k| {
                let shape = coded_shapes[k];
                let selector = readings.selector(&bytes, shape);
                let bits = logistic_mix_bits_with_second_selector(&bytes, true, &[], prior, &selector.values, selector.sets)
                    + selector.side_bits as f64;
                let bpb = bits / len.max(1) as f64;
                println!(
                    "GRID {name} {} {} {} {} {} {bpb:.6} {plain:.6} {grid_r} {grid_age:?}",
                    shape.byte_bits, shape.power_bins, shape.age_bins, selector.sets, selector.side_bits
                );
                bpb
            });
            eprintln!("  {} shapes in {:.1} s", coded.len(), t0.elapsed().as_secs_f64());
            continue;
        }
        if shipped {
            // The codings `trex compress` offers and the two its second mixer
            // is weighed against, and nothing the other arms read, so no
            // selector of this harness's own is built.
            let per_byte = |bits: f64| bits / len.max(1) as f64;
            let plain = per_byte(logistic_mix_bits_with(&bytes, true, &[], prior));
            let second = per_byte(trex::seam::second_mixer_bits_with(&bytes, prior));
            let nothing = vec![0u16; len];
            let one =
                per_byte(logistic_mix_bits_with_mixers_beside(&bytes, true, &[], prior, &[(nothing.as_slice(), 1)]));
            let rhythm = trex::seam::rhythm_selector(&bytes);
            let three = per_byte(
                logistic_mix_bits_with_mixers_beside(
                    &bytes,
                    true,
                    &[],
                    prior,
                    &[(nothing.as_slice(), 1), (rhythm.values.as_slice(), rhythm.sets)],
                ) + rhythm.side_bits as f64,
            );
            println!("{name:<22} {len:>10}  {plain:>7.4}  {second:>10.4}  {one:>10.4}  {three:>13.4}");
            continue;
        }

        let t0 = Instant::now();
        let (toks, ages, powers) = token_regimes(&bytes, &periods, r, which);
        let pw_bins = quantile_bins(&powers);
        let age = per_byte(len, &toks, &ages, 0, delay);
        let age_shift = shifted(&age);
        let power = per_byte(len, &toks, &pw_bins, 0, delay);
        let power_shift = shifted(&power);
        let before = |t: usize| if t == 0 { 0u16 } else { u16::from(bytes[t - 1]) };
        let nib: Vec<u16> = (0..len).map(|t| before(t) >> 4).collect();
        let nib_gd: Vec<u16> = (0..len).map(|t| nib[t] * BINS + age[t]).collect();
        let nib_gds: Vec<u16> = (0..len).map(|t| nib[t] * BINS + age_shift[t]).collect();
        let nib_pw: Vec<u16> = (0..len).map(|t| nib[t] * BINS + power[t]).collect();
        let nib_pws: Vec<u16> = (0..len).map(|t| nib[t] * BINS + power_shift[t]).collect();
        let byte_gd: Vec<u16> = (0..len).map(|t| before(t) * BINS + age[t]).collect();
        let byte_gds: Vec<u16> = (0..len).map(|t| before(t) * BINS + age_shift[t]).collect();
        let nib_pw_gd: Vec<u16> =
            (0..len).map(|t| (nib[t] * BINS + power[t]) * BINS + age[t]).collect();
        let nib_pw_gds: Vec<u16> =
            (0..len).map(|t| (nib[t] * BINS + power[t]) * BINS + age_shift[t]).collect();
        let selectors_s = t0.elapsed().as_secs_f64();
        eprintln!("  selectors in {selectors_s:.1} s");

        let bpb = |bits: f64| bits / len.max(1) as f64;
        if let Some(rounds) = time {
            // The shipped mixer and the second mixer beside it, alternately and
            // each round led by the other, over the same bytes and the same
            // selector. The clock holds one coder call and nothing else: the
            // selectors are read above, once, and their cost is printed apart.
            let sets = 16 * usize::from(BINS) * usize::from(BINS);
            let mut secs: [Vec<f64>; 2] = [Vec::with_capacity(rounds), Vec::with_capacity(rounds)];
            let mut bits: [Option<f64>; 2] = [None, None];
            for round in 0..rounds {
                let order = if round % 2 == 0 { [0usize, 1] } else { [1, 0] };
                for arm in order {
                    let t = Instant::now();
                    let coded = if arm == 0 {
                        logistic_mix_bits_with(&bytes, true, &[], prior)
                    } else {
                        logistic_mix_bits_with_second_selector(&bytes, true, &[], prior, &nib_pw_gd, sets)
                    };
                    secs[arm].push(t.elapsed().as_secs_f64());
                    match bits[arm] {
                        None => bits[arm] = Some(coded),
                        Some(first) if first.to_bits() == coded.to_bits() => {}
                        Some(first) => {
                            eprintln!("{name}: arm {arm} coded {first} bits and then {coded}; the coder is not deterministic");
                            std::process::exit(1);
                        }
                    }
                }
            }
            let [Some(bits_shipped), Some(bits_beside)] = bits else {
                unreachable!("--time refuses zero rounds, so both arms ran")
            };
            let [mut plain_s, mut beside_s] = secs;
            plain_s.sort_by(f64::total_cmp);
            beside_s.sort_by(f64::total_cmp);
            let (p, b) = (plain_s[rounds / 2], beside_s[rounds / 2]);
            println!(
                "{name:<22} {len:>10}  selectors {selectors_s:.2} s once; shipped {p:.3} s ({:.3}-{:.3}), \
                 beside npg {b:.3} s ({:.3}-{:.3}), {:.3}x; bits per byte {:.4} and {:.4}",
                plain_s[0],
                plain_s[rounds - 1],
                beside_s[0],
                beside_s[rounds - 1],
                b / p,
                bpb(bits_shipped),
                bpb(bits_beside)
            );
            continue;
        }
        let t1 = Instant::now();
        let plain = bpb(logistic_mix_bits_with(&bytes, true, &[], prior));
        let arm = |sel: &[u16], sets: usize| {
            bpb(logistic_mix_bits_with_selector(&bytes, true, &[], prior, sel, sets))
        };
        let nibs = 16;
        let a_nib = arm(&nib, nibs);
        let crossed = nibs * usize::from(BINS);
        let (a_ng, a_ngs) = (arm(&nib_gd, crossed), arm(&nib_gds, crossed));
        let (a_np, a_nps) = (arm(&nib_pw, crossed), arm(&nib_pws, crossed));
        let wide = 256 * usize::from(BINS);
        let (a_bg, a_bgs) = (arm(&byte_gd, wide), arm(&byte_gds, wide));
        let both = crossed * usize::from(BINS);
        let (a_npg, a_npgs) = (arm(&nib_pw_gd, both), arm(&nib_pw_gds, both));
        let beside = |sel: &[u16], sets: usize| {
            bpb(logistic_mix_bits_with_second_selector(&bytes, true, &[], prior, sel, sets))
        };
        let (b_npg, b_npgs) = (beside(&nib_pw_gd, both), beside(&nib_pw_gds, both));
        let b_np = beside(&nib_pw, crossed);
        let b_nib = beside(&nib, nibs);
        let nothing = vec![0u16; len];
        let b_one = beside(&nothing, 1);
        let shipped = bpb(trex::seam::second_mixer_bits_with(&bytes, prior));
        let rhythm = trex::seam::rhythm_selector(&bytes);
        let three = bpb(
            logistic_mix_bits_with_mixers_beside(
                &bytes,
                true,
                &[],
                prior,
                &[(nothing.as_slice(), 1), (rhythm.values.as_slice(), rhythm.sets)],
            ) + rhythm.side_bits as f64,
        );
        eprintln!("  mixer arms in {:.1} s", t1.elapsed().as_secs_f64());
        println!(
            "{name:<22} {len:>10}  {plain:>7.4}  {a_nib:>7.4}  {a_ng:>7.4} {a_ngs:>7.4}  {a_np:>7.4} {a_nps:>7.4}  \
             {a_bg:>8.4} {a_bgs:>8.4}  {a_npg:>9.4} {a_npgs:>8.4}  {b_npg:>11.4} {b_npgs:>8.4}  {b_np:>10.4}  \
             {b_nib:>10.4}  {b_one:>10.4}  {shipped:>10.4}  {three:>13.4}"
        );
    }
    if unreadable > 0 {
        eprintln!("{unreadable} of {} inputs could not be read", args.len());
        std::process::exit(1);
    }
}
