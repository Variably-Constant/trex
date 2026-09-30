//! Whether the group delay makes trex's own coder shorter, read on real files
//! in bits per byte, and in which of the two ways a coder can take it.
//!
//! The group delay was measured carrying information about the next token that
//! neither the current token, the power, nor the token before it holds -
//! `what_the_group_delay_predicts`. Information a coder cannot use is an
//! observation, not a component, so the coder is where the question of where
//! it belongs is settled.
//!
//! A coder can take a reading two ways, and they are not the same test.
//!
//! Folded into its context keys - `seam::ppm_byte_bits_with_side` - a reading
//! splits every context it touches and divides its counts. That is
//! multiplicative: a reading worth less than the dilution it causes makes the
//! coder worse whatever it carries.
//!
//! Mixed in as a predictor of its own - `seam::logistic_mix_bits_with_aux`,
//! the production PAQ-style mixer `trex compress` runs - a reading gets a count
//! table and a mixer weight that starts at zero, so the mixer raises it only
//! where it predicts better than the models already there. That is additive,
//! and it is the direct test of whether the reading stacks.
//!
//! Both read leak-free, since byte `t` is coded from what a decoder has before
//! it. At the token grain the reading is the bank's state after the last token
//! that ended before `t`, never the token covering `t`, whose extent is decided
//! by bytes not yet coded - the same rule the mixer's shape input follows. At
//! the byte grain it is the bank's state read before byte `t` goes in. The
//! group delay is binned on fixed edges across its natural range, zero to the
//! bank's memory, so its bins read no statistic of the stream. The power has no
//! natural range, so its bins are quantiles of the stream's own distribution;
//! it is here as the control the group delay is stacked on.
//!
//! Every reading is also given moved half the stream along - the same values,
//! none of their position - and a reading is information rather than a second
//! context of any kind when it codes shorter than its own shift.
//!
//! The mixer runs with its orbit models on and no baked prior or warm seed, so
//! the only difference between two arms is the one auxiliary context.
//!
//! `--r` sets the bank's pole radius, whose memory is about `r / (1 - r)`
//! tokens, and `--age dom` reads the loudest band's own group delay in place of
//! the power-weighted mean: `what_the_group_delay_predicts` finds both carrying
//! more about the next token at a shorter memory than the bank ships with.
//! `--delay D` holds every token-grain reading back until `D` bytes past the
//! token's end: a recognizer can read past the token it emits
//! (`does_the_lexer_read_ahead`), so a gain that survives the delay is not one
//! carried by that reach.
//!
//! Every input is a real file somebody produced for another purpose.
//!
//! Run: `cargo run --release --example does_the_group_delay_help_the_coder -- [--r <pole radius>] [--age mean|dom] [--delay <bytes>] <file>...`

use std::path::Path;
use std::time::Instant;

use trex::resonator::{Bank, DEFAULT_R, default_periods};
use trex::seam::{
    PreloadTable, logistic_mix_bits_with, logistic_mix_bits_with_aux,
    logistic_mix_bits_with_shared_aux, ppm_byte_bits, ppm_byte_bits_with_side,
};

/// The PPM order the multiplicative arms are read at.
const ORDER: usize = 4;

/// Bins a reading is cut into.
const BINS: u32 = 8;

/// `v` moved half its length along, which keeps every value and none of its
/// position.
fn shifted<T: Copy>(v: &[T]) -> Vec<T> {
    let n = v.len();
    let half = n / 2;
    (0..n).map(|i| v[(i + half) % n]).collect()
}

/// Each position's quantile bin in `score`.
fn quantile_bins(score: &[f32]) -> Vec<u32> {
    let n = score.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_unstable_by(|&a, &b| score[a].total_cmp(&score[b]));
    let mut out = vec![0u32; n];
    for (r, &i) in order.iter().enumerate() {
        out[i] = ((r * BINS as usize) / n.max(1)).min(BINS as usize - 1) as u32;
    }
    out
}

/// One context value from parts, distinct for every distinct tuple and never
/// zero, so a byte with no reading yet keeps a context of its own.
fn key(parts: &[u64]) -> u64 {
    let mut h = 0x243F_6A88_85A3_08D3u64;
    for &p in parts {
        h = (h ^ p).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
    h | 1
}

/// The group delay's fixed bin: zero to the bank's memory, the top bin taking
/// everything past it.
fn age_bin(age: f32, span: f32) -> u32 {
    ((age / span * BINS as f32) as u32).min(BINS - 1)
}

/// Which age a reading takes from the bank.
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

/// The age `which` names and the log of the total power, as the bank stands.
fn reading(bank: &Bank, gd: &mut [f32], pw: &mut [f32], which: Age) -> (f32, f32) {
    bank.read_into(gd, pw);
    let total: f32 = pw.iter().sum();
    let age = match which {
        Age::Mean if total > f32::EPSILON => {
            gd.iter().zip(pw.iter()).map(|(g, p)| g * p).sum::<f32>() / total
        }
        Age::Mean => 0.0,
        Age::Dominant => pw
            .iter()
            .zip(gd.iter())
            .max_by(|a, b| a.0.total_cmp(b.0))
            .map_or(0.0, |(_, &g)| g),
    };
    (age, (total + f32::EPSILON).ln())
}

/// The bank's pole radius, the age it is read for and the delay a token-grain
/// reading is held back by, from `--r <radius>`, `--age mean|dom` and
/// `--delay <bytes>` at the front of `args`, which are taken off it, leaving
/// the files. A value that does not parse is refused rather than defaulted,
/// since a table labelled with one setting and read at another is the failure
/// this would otherwise hide.
fn options(args: &mut Vec<String>) -> (f32, Age, usize) {
    let (mut r, mut age, mut delay) = (DEFAULT_R, Age::Mean, 0usize);
    loop {
        match args.first().map(String::as_str) {
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
            _ => return (r, age, delay),
        }
    }
}

/// Per significant token: its kind code packed dense, and the bank's group
/// delay bin and log power after it.
fn token_readings(
    input: &[u8],
    periods: &[u16],
    r: f32,
    which: Age,
) -> (Vec<trex::token::Token>, Vec<u32>, Vec<u32>, Vec<f32>) {
    let toks: Vec<trex::token::Token> =
        trex::lexer::lex(input).into_iter().filter(|t| t.is_significant()).collect();
    let raw: Vec<u32> = toks.iter().map(|t| t.kind.code()).collect();
    let mut distinct = raw.clone();
    distinct.sort_unstable();
    distinct.dedup();
    let kinds: Vec<u32> = raw.iter().map(|s| distinct.partition_point(|d| d < s) as u32).collect();
    let span = r / (1.0 - r);
    let mut bank = Bank::new(distinct.len(), periods, r);
    let mut gd = vec![0.0f32; periods.len()];
    let mut pw = vec![0.0f32; periods.len()];
    let mut ages = Vec::with_capacity(kinds.len());
    let mut powers = Vec::with_capacity(kinds.len());
    for &k in &kinds {
        bank.push(k);
        let (age, lp) = reading(&bank, &mut gd, &mut pw, which);
        ages.push(age_bin(age, span));
        powers.push(lp);
    }
    (toks, kinds, ages, powers)
}

/// A context a byte, built from the last significant token that ended more
/// than `delay` bytes before it: `part(c)` for that token, or a context of its
/// own where none has.
fn per_byte(len: usize, toks: &[trex::token::Token], delay: usize, part: impl Fn(usize) -> u64) -> Vec<u64> {
    let mut out = vec![0u64; len];
    let mut done = 0usize;
    for (t, slot) in out.iter_mut().enumerate() {
        // Strictly before `t`: a token ending at `t` had its end decided by the
        // byte being coded. The delay holds the reading further back still,
        // past what a recognizer can read beyond the token it emits.
        while done < toks.len() && toks[done].end() + delay < t {
            done += 1;
        }
        *slot = if done == 0 { 1 } else { part(done - 1) };
    }
    out
}

fn short(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let (r, which, delay) = options(&mut args);
    if args.is_empty() {
        eprintln!(
            "usage: does_the_group_delay_help_the_coder [--r <pole radius>] [--age mean|dom] [--delay <bytes>] <file>..."
        );
        std::process::exit(2);
    }
    let periods = default_periods(32);
    let span = r / (1.0 - r);

    println!("bits per byte. `~` is the reading moved half the stream along.");
    println!("the bank at r {r}, a memory of about {span:.0} tokens; gd is {} group delay.", which.name());
    println!("tok: the bank over tokens, read after the last token ending {delay} bytes or more before the byte.");
    println!("byte: the bank over byte classes, read before the byte goes in.");
    println!();
    println!("additive: the production mixer with one auxiliary predictor");
    println!(
        "{:<22} {:>10}  {:>7}  {:>7} {:>7}  {:>7} {:>7}  {:>7}  {:>7} {:>7}",
        "input", "bytes", "plain", "tok gd", "~", "byte gd", "~", "tok pw", "pw+gd", "~"
    );

    let mut split_rows: Vec<String> = Vec::new();
    let mut shared_rows: Vec<String> = Vec::new();
    // An input that cannot be read is a row missing from the table, and a
    // table missing a row reads as complete. So a run with one exits failing.
    let mut unreadable = 0usize;
    for (n, arg) in args.iter().enumerate() {
        let path = Path::new(arg);
        let name = short(path);
        eprintln!("[{}/{}] {name}", n + 1, args.len());
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                println!("{name:<22} unreadable: {e}");
                unreadable += 1;
                continue;
            }
        };
        let len = bytes.len();

        let t0 = Instant::now();
        let (toks, kinds, ages, powers) = token_readings(&bytes, &periods, r, which);
        let pw_bins = quantile_bins(&powers);
        let ages_shift = shifted(&ages);
        let tok_gd = per_byte(len, &toks, delay, |c| key(&[u64::from(kinds[c]), u64::from(ages[c])]));
        let tok_gds =
            per_byte(len, &toks, delay, |c| key(&[u64::from(kinds[c]), u64::from(ages_shift[c])]));
        let tok_pw = per_byte(len, &toks, delay, |c| key(&[u64::from(kinds[c]), u64::from(pw_bins[c])]));
        let tok_both = per_byte(len, &toks, delay, |c| {
            key(&[u64::from(kinds[c]), u64::from(pw_bins[c]), u64::from(ages[c])])
        });
        let tok_boths = per_byte(len, &toks, delay, |c| {
            key(&[u64::from(kinds[c]), u64::from(pw_bins[c]), u64::from(ages_shift[c])])
        });

        // The byte grain: read before each byte goes in, keyed with the byte
        // before it so it is an order-one context split by the rhythm's age.
        let mut bank = Bank::new(trex::spectral::N_CLASSES, &periods, r);
        let mut gd = vec![0.0f32; periods.len()];
        let mut pw = vec![0.0f32; periods.len()];
        let mut byte_age = vec![0u32; len];
        for (t, &b) in bytes.iter().enumerate() {
            let (age, _) = reading(&bank, &mut gd, &mut pw, which);
            byte_age[t] = age_bin(age, span);
            bank.push(trex::spectral::classify(b) as u32);
        }
        let byte_age_shift = shifted(&byte_age);
        let prev = |t: usize| if t == 0 { 256 } else { u64::from(bytes[t - 1]) };
        let byte_gd: Vec<u64> = (0..len).map(|t| key(&[prev(t), u64::from(byte_age[t])])).collect();
        let byte_gds: Vec<u64> =
            (0..len).map(|t| key(&[prev(t), u64::from(byte_age_shift[t])])).collect();
        eprintln!("  readings in {:.1} s", t0.elapsed().as_secs_f64());

        let bpb = |bits: f64| bits / len.max(1) as f64;
        let t1 = Instant::now();
        let none: Option<&[PreloadTable]> = None;
        let plain = bpb(logistic_mix_bits_with(&bytes, true, &[], none));
        eprintln!("  mixer plain in {:.1} s", t1.elapsed().as_secs_f64());
        let mix = |aux: &[u64]| bpb(logistic_mix_bits_with_aux(&bytes, true, &[], none, aux));
        let t2 = Instant::now();
        let (a_tok, a_toks) = (mix(&tok_gd), mix(&tok_gds));
        let (a_byte, a_bytes) = (mix(&byte_gd), mix(&byte_gds));
        let a_pw = mix(&tok_pw);
        let (a_both, a_boths) = (mix(&tok_both), mix(&tok_boths));
        eprintln!("  mixer arms in {:.1} s", t2.elapsed().as_secs_f64());
        println!(
            "{name:<22} {len:>10}  {plain:>7.4}  {a_tok:>7.4} {a_toks:>7.4}  {a_byte:>7.4} {a_bytes:>7.4}  \
             {a_pw:>7.4}  {a_both:>7.4} {a_boths:>7.4}"
        );

        // The same token-grain readings through one weight shared across every
        // weight set, which lowers what an auxiliary input costs while it is
        // being learned and gives up trusting it differently by regime.
        let t_shared = Instant::now();
        let shared = |aux: &[u64]| {
            bpb(logistic_mix_bits_with_shared_aux(&bytes, true, &[], none, aux))
        };
        let (s_tok, s_toks) = (shared(&tok_gd), shared(&tok_gds));
        let (s_both, s_boths) = (shared(&tok_both), shared(&tok_boths));
        eprintln!("  shared arms in {:.1} s", t_shared.elapsed().as_secs_f64());
        shared_rows.push(format!(
            "{name:<22} {len:>10}  {plain:>7.4}  {s_tok:>7.4} {s_toks:>7.4}  {s_both:>7.4} {s_boths:>7.4}"
        ));

        // The multiplicative contrast: the byte-grain age folded into PPM's two
        // lowest orders.
        let t3 = Instant::now();
        let side: Vec<u8> = byte_age.iter().map(|&a| a as u8).collect();
        let side_shift = shifted(&side);
        let p_plain = bpb(ppm_byte_bits(&bytes, ORDER, false));
        let p_gd = bpb(ppm_byte_bits_with_side(&bytes, ORDER, &side, 2));
        let p_gds = bpb(ppm_byte_bits_with_side(&bytes, ORDER, &side_shift, 2));
        eprintln!("  ppm arms in {:.1} s", t3.elapsed().as_secs_f64());
        split_rows.push(format!("{name:<22} {len:>10}  {p_plain:>7.4}  {p_gd:>7.4} {p_gds:>7.4}"));
    }

    println!();
    println!("multiplicative: PPM order {ORDER}, the byte-grain age folded into its two lowest orders");
    println!("{:<22} {:>10}  {:>7}  {:>7} {:>7}", "input", "bytes", "plain", "byte gd", "~");
    for row in &split_rows {
        println!("{row}");
    }

    println!();
    println!("additive, one shared weight: the token-grain readings with a single mixer weight");
    println!(
        "{:<22} {:>10}  {:>7}  {:>7} {:>7}  {:>7} {:>7}",
        "input", "bytes", "plain", "tok gd", "~", "pw+gd", "~"
    );
    for row in &shared_rows {
        println!("{row}");
    }
    if unreadable > 0 {
        eprintln!("{unreadable} of {} inputs could not be read", args.len());
        std::process::exit(1);
    }
}
