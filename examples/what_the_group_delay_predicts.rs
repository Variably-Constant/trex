//! Whether the resonator's readings carry predictive structure the token and
//! the power do not, read on real files by the only target that needs no
//! labels: the next token.
//!
//! Three quantities come off the bank, and they are not two readings of one
//! thing. The power is how much rhythm a period carries. The group delay is how
//! long ago that rhythm's energy was laid down - an age. And the group delay's
//! rate of change is the reassignment literature's mixed partial derivative of
//! phase (Fitz and Fulop, arXiv 0903.3080, equations 107 to 110), which reads
//! near zero where the stream is a steady rhythm and near one in the wake of a
//! transient: it classifies each stretch as sinusoid-like or impulse-like. The
//! paper's own statement of why: near an impulsive component "all times, t,
//! should be mapped to approximately the same reassigned time", so the
//! reassigned time stays fixed while t moves, and the group delay - which is
//! t less the reassigned time - rises one position per position.
//!
//! Scoring them as boundary points found the rate diluting the power and the
//! seam. That was asking a region classifier to be a point detector - the rate
//! stays near one for the bank's whole memory after a transient, so as a point
//! it smears by construction - and the question it failed is not the one it
//! answers. The question every structural reading answers, whatever its form,
//! is whether it says something about what comes next. That needs no ground
//! truth and it is the principle the seam itself runs on: a boundary is where
//! the past stops predicting.
//!
//! So each reading is binned by quantile and the conditional entropy of the next
//! token is read under contexts that add one reading at a time to the token
//! itself. Conditioning on anything lowers an empirical entropy through the
//! finite sample alone, so every reading is also added shifted half the stream
//! along - the same values and the same bins, none of their position - and a
//! reading's information is what it removes beyond what its shifted copy
//! removes. `gd | pw` is the group delay's information once the power is
//! already known, which is the direct test of whether it stacks.
//!
//! Every input is a real file somebody produced for another purpose.
//!
//! Run: `cargo run --release --example what_the_group_delay_predicts -- <file>...`

use std::path::Path;
use std::time::Instant;

use trex::resonator::{Bank, DEFAULT_R, default_periods};

/// Quantile bins a reading is cut into. Eight keeps a context of the token and
/// two readings at a few thousand cells, which every corpus here fills.
const BINS: usize = 8;

/// Each position's quantile bin in `score`, from `0` to `BINS - 1`.
fn bins(score: &[f32]) -> Vec<u32> {
    let n = score.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_unstable_by(|&a, &b| score[a].total_cmp(&score[b]));
    let mut out = vec![0u32; n];
    for (r, &i) in order.iter().enumerate() {
        out[i] = ((r * BINS) / n.max(1)).min(BINS - 1) as u32;
    }
    out
}

/// `v` moved half its length along, which keeps every value and none of its
/// position.
fn shifted<T: Copy>(v: &[T]) -> Vec<T> {
    let n = v.len();
    let half = n / 2;
    (0..n).map(|i| v[(i + half) % n]).collect()
}

/// One component of a context: a value at every position, and how many values
/// it can take.
type Part<'a> = (&'a [u32], usize);

/// The entropy in bits of the next symbol given a context built from `parts`,
/// over positions `from..n - 1`. The context at `t` is every part's value at
/// `t` read as one mixed-radix number, so its cardinality is the product of
/// the parts' and the counts are that times the alphabet.
fn conditional_entropy(symbols: &[u32], alphabet: usize, parts: &[Part<'_>], from: usize) -> f64 {
    let n = symbols.len();
    if n < from + 2 {
        return 0.0;
    }
    let contexts: usize = parts.iter().map(|&(_, card)| card).product();
    let mut joint = vec![0u32; contexts * alphabet];
    let mut marginal = vec![0u32; contexts];
    let mut total = 0u64;
    for t in from..n - 1 {
        let mut ctx = 0usize;
        for &(values, card) in parts {
            ctx = ctx * card + values[t] as usize;
        }
        let y = symbols[t + 1] as usize;
        joint[ctx * alphabet + y] += 1;
        marginal[ctx] += 1;
        total += 1;
    }
    let total = total as f64;
    let mut h = 0.0f64;
    for ctx in 0..contexts {
        let m = f64::from(marginal[ctx]);
        if m == 0.0 {
            continue;
        }
        for y in 0..alphabet {
            let c = f64::from(joint[ctx * alphabet + y]);
            if c > 0.0 {
                h -= (c / total) * (c / m).log2();
            }
        }
    }
    h
}

fn short(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // `--r <pole radius>` sets the bank's memory, about `1 / (1 - r)` tokens,
    // so the reading can be taken at the timescale it lives at rather than the
    // one the bank ships with. An unreadable radius is refused rather than
    // defaulted, since a table labeled with one memory and read at another is
    // the failure this would otherwise hide.
    let mut r = DEFAULT_R;
    if args.first().is_some_and(|a| a == "--r") {
        let Some(value) = args.get(1) else {
            eprintln!("--r needs a pole radius, for example --r 0.99");
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
    if args.is_empty() {
        eprintln!("usage: what_the_group_delay_predicts [--r <pole radius>] <file>...");
        std::process::exit(2);
    }
    let periods = default_periods(32);
    // The first memories of the bank fill from a zero state.
    let warm = (3.0 / (1.0 - r)) as usize;

    println!("bits of the next token each reading removes beyond its own shifted copy,");
    println!("given the current token; `gd | pw` is the group delay's once the power is known.");
    println!("rate is the group delay's change, the impulse-or-steady reading. r {r}.");
    println!("prev is the token before the current one; `gd|prev` is what the group delay");
    println!("adds once a two-token context is known, which no short n-gram can supply.");
    println!();
    println!("dom is the loudest rhythm's own age rather than the power-weighted mean.");
    println!();
    println!(
        "{:<22} {:>9}  {:>6}  {:>7} {:>7} {:>7}  {:>8} {:>9}  {:>7} {:>8} {:>8}  {:>7} {:>8}",
        "input", "sig toks", "H(x'|x)", "pw", "gd", "rate", "gd|pw", "rate|pw", "prev",
        "pw|prev", "gd|prev", "dom", "dom|pw"
    );

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
        let toks = trex::lexer::lex(&bytes);
        let raw: Vec<u32> =
            toks.iter().filter(|t| t.is_significant()).map(|t| t.kind.code()).collect();
        // Packed to the codes present: the bank's state and its cost a push are
        // both the alphabet times the band count.
        let mut distinct = raw.clone();
        distinct.sort_unstable();
        distinct.dedup();
        let symbols: Vec<u32> =
            raw.iter().map(|s| distinct.partition_point(|d| d < s) as u32).collect();
        let alphabet = distinct.len();

        let t0 = Instant::now();
        let mut bank = Bank::new(alphabet, &periods, r);
        let mut gd = vec![0.0f32; periods.len()];
        let mut pw = vec![0.0f32; periods.len()];
        let mut prev_gd = vec![0.0f32; periods.len()];
        let mut log_power = vec![0.0f32; symbols.len()];
        let mut age = vec![0.0f32; symbols.len()];
        let mut rate = vec![0.0f32; symbols.len()];
        // The age of the loudest rhythm alone. The power-weighted mean blends
        // every rhythm's age into one number; a stream carrying a strong record
        // and a weak word rhythm reads mostly the record's age there, but the
        // blend still carries the word rhythm's share, and the loudest band's
        // own age is the sharper reading of whether the rhythm that matters is
        // fresh or fading.
        let mut dominant_age = vec![0.0f32; symbols.len()];
        for (t, &s) in symbols.iter().enumerate() {
            bank.push(s);
            bank.read_into(&mut gd, &mut pw);
            let total: f32 = pw.iter().sum();
            log_power[t] = (total + f32::EPSILON).ln();
            // The power-weighted mean group delay: the age of the rhythm that
            // carries most of the energy, rather than of a period that carries
            // none.
            age[t] = if total > f32::EPSILON {
                gd.iter().zip(&pw).map(|(g, p)| g * p).sum::<f32>() / total
            } else {
                0.0
            };
            let mut d = 0.0f32;
            for (&g, pg) in gd.iter().zip(prev_gd.iter_mut()) {
                if t > 0 {
                    d += (g - *pg).abs();
                }
                *pg = g;
            }
            rate[t] = d / periods.len() as f32;
            dominant_age[t] = pw
                .iter()
                .zip(&gd)
                .max_by(|a, b| a.0.total_cmp(b.0))
                .map_or(0.0, |(_, &g)| g);
        }
        eprintln!("  bank over {} symbols in {:.1} s", symbols.len(), t0.elapsed().as_secs_f64());

        let t1 = Instant::now();
        let (bp, bg, br) = (bins(&log_power), bins(&age), bins(&rate));
        let (sp, sg, sr) = (shifted(&bp), shifted(&bg), shifted(&br));
        // The token before the current one. The bank at `t` has read every
        // token up to `t`, so a reading could carry nothing but a longer
        // n-gram context in disguise; this is that context itself, so what the
        // readings add beyond it is what no short context holds.
        let prev: Vec<u32> =
            std::iter::once(0).chain(symbols.iter().copied()).take(symbols.len()).collect();
        let sprev = shifted(&prev);
        let h = |parts: &[Part<'_>]| conditional_entropy(&symbols, alphabet, parts, warm);
        let x: Part<'_> = (symbols.as_slice(), alphabet);
        let base = h(&[x]);
        // Each reading's information beyond its shifted copy, given the token.
        let info_pw = h(&[x, (&sp, BINS)]) - h(&[x, (&bp, BINS)]);
        let info_gd = h(&[x, (&sg, BINS)]) - h(&[x, (&bg, BINS)]);
        let info_rate = h(&[x, (&sr, BINS)]) - h(&[x, (&br, BINS)]);
        // Given the token and the power, what each still adds.
        let gd_given_pw = h(&[x, (&bp, BINS), (&sg, BINS)]) - h(&[x, (&bp, BINS), (&bg, BINS)]);
        let rate_given_pw =
            h(&[x, (&bp, BINS), (&sr, BINS)]) - h(&[x, (&bp, BINS), (&br, BINS)]);
        // Given the token and the one before it, what each still adds.
        let info_prev = h(&[x, (&sprev, alphabet)]) - h(&[x, (&prev, alphabet)]);
        let pw_given_prev =
            h(&[x, (&prev, alphabet), (&sp, BINS)]) - h(&[x, (&prev, alphabet), (&bp, BINS)]);
        let gd_given_prev =
            h(&[x, (&prev, alphabet), (&sg, BINS)]) - h(&[x, (&prev, alphabet), (&bg, BINS)]);
        // The loudest rhythm's own age, alone and once the power is known.
        let bd = bins(&dominant_age);
        let sd = shifted(&bd);
        let info_dom = h(&[x, (&sd, BINS)]) - h(&[x, (&bd, BINS)]);
        let dom_given_pw = h(&[x, (&bp, BINS), (&sd, BINS)]) - h(&[x, (&bp, BINS), (&bd, BINS)]);
        eprintln!("  entropies in {:.1} s", t1.elapsed().as_secs_f64());

        println!(
            "{name:<22} {:>9}  {base:>6.3}  {info_pw:>7.4} {info_gd:>7.4} {info_rate:>7.4}  \
             {gd_given_pw:>8.4} {rate_given_pw:>9.4}  {info_prev:>7.4} {pw_given_prev:>8.4} \
             {gd_given_prev:>8.4}  {info_dom:>7.4} {dom_given_pw:>8.4}",
            symbols.len()
        );
    }
    if unreadable > 0 {
        eprintln!("{unreadable} of {} inputs could not be read", args.len());
        std::process::exit(1);
    }
}
