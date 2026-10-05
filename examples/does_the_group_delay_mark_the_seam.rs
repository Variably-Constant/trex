//! Whether the resonator's group delay marks the structure the seam marks,
//! read on real files through the agreement the context already scores its
//! grains by.
//!
//! The group delay is a position - where inside the bank's memory a band's
//! energy is - and it separates a ragged table from prose and code by 13 to
//! 26 times at the band matching the table's field width, where the power
//! ranks that same table last. A position is what a boundary is, so the
//! question is whether its changes land where structure changes; and the seam
//! axis is what answers "where does structure change" in this crate.
//!
//! The test is the one `trex::context::agreement` already applies to the
//! regime, shape and seam grains: for each supertoken, the offset in
//! significant tokens from its start to a grain's nearest boundary, and the
//! share of units whose offset is the modal one for their role. A grain
//! reading the construct lands at one offset per role, and a grain that reads
//! no construct spreads. Reusing it rather than inventing a distance means the
//! group delay is scored exactly as the grains it is compared with are.
//!
//! Two things that score would otherwise read as structure are held fixed.
//!
//! Density: a dense cut set is near every unit start whatever it reads, so
//! every set compared here holds the same number of cuts - the smallest
//! natural count among them, each set keeping its strongest.
//!
//! Resolution: a reading taken every few tokens lands at an arbitrary offset
//! modulo that stride, which spreads the offsets as surely as reading no
//! construct does. So the bank is read at every token, which is what `resonator::Bank`
//! is for, and the seam is read over the same token-kind stream.
//!
//! Beside the seam, two references. The power's own changes, since the claim
//! under test is that the group delay carries what the power does not. And
//! cuts at random token positions at the same count, which is what the share
//! reads when nothing is read at all.
//!
//! Weaker is not the same as contained in. Two signals that are each weak can
//! each carry what the other lacks, and then together beat either, so a
//! second table asks whether the group delay adds to what the power or the
//! seam already reads - not whether it is stronger on its own - each summed
//! with it by rank, against each alone, and
//! against each summed with the group delay moved half the stream along - which
//! keeps its values and its smoothness and loses only where its changes land.
//! It also gives the rank correlation between the group delay's changes and the
//! power's, which says directly how much of one the other already holds.
//!
//! Every input is a real file somebody produced for another purpose.
//!
//! Run: `cargo run --release --example does_the_group_delay_mark_the_seam -- <file>...`

use std::path::Path;
use std::time::Instant;

use trex::context::{ContextField, agreement};
use trex::resonator::{Bank, DEFAULT_R, default_periods};
use trex::seam::SeamConfig;

/// Two change points closer than half the bank's memory are one change to it,
/// so the strongest changes are taken at least this far apart.
fn separation(r: f32) -> usize {
    ((0.5 / (1.0 - r)).round() as usize).max(1)
}

/// splitmix64. Its finalizer avalanches, so consecutive draws carry no stride
/// a nearest-offset reading could take for structure - the defect a
/// multiply-add generator once planted in every corpus built from it.
fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Every separated maximum of `score`, strongest first: a position is taken
/// when nothing within `radius` of it was taken already.
fn top_separated(score: &[f32], radius: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..score.len()).filter(|&i| score[i] > 0.0).collect();
    order.sort_unstable_by(|&a, &b| score[b].total_cmp(&score[a]));
    let mut taken = vec![false; score.len()];
    let mut out = Vec::new();
    for i in order {
        if taken[i] {
            continue;
        }
        out.push(i);
        let lo = i.saturating_sub(radius);
        let hi = (i + radius + 1).min(score.len());
        for t in &mut taken[lo..hi] {
            *t = true;
        }
    }
    out
}

/// Each position's fractional rank in `score`, in `[0, 1)`. Scale-free, so two
/// signals in different units can be summed with neither winning by its units
/// - the group delay is in positions and the power's change is on a log.
fn ranks(score: &[f32]) -> Vec<f32> {
    let n = score.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_unstable_by(|&a, &b| score[a].total_cmp(&score[b]));
    let mut out = vec![0.0f32; n];
    let denom = n.max(1) as f32;
    for (r, &i) in order.iter().enumerate() {
        out[i] = r as f32 / denom;
    }
    out
}

/// Spearman's rank correlation over `from..`: Pearson's over the two rank
/// vectors, skipping the prefix where the bank is still filling.
fn spearman(a: &[f32], b: &[f32], from: usize) -> f64 {
    let a = &a[from.min(a.len())..];
    let b = &b[from.min(b.len())..];
    let n = a.len().min(b.len()) as f64;
    if n < 2.0 {
        return 0.0;
    }
    let ma = a.iter().map(|&x| f64::from(x)).sum::<f64>() / n;
    let mb = b.iter().map(|&x| f64::from(x)).sum::<f64>() / n;
    let (mut cov, mut va, mut vb) = (0.0f64, 0.0f64, 0.0f64);
    for (&x, &y) in a.iter().zip(b) {
        let (dx, dy) = (f64::from(x) - ma, f64::from(y) - mb);
        cov += dx * dy;
        va += dx * dx;
        vb += dy * dy;
    }
    if va <= 0.0 || vb <= 0.0 { 0.0 } else { cov / (va * vb).sqrt() }
}

/// The positionwise sum of two rank vectors.
fn plus(a: &[f32], b: &[f32]) -> Vec<f32> {
    a.iter().zip(b).map(|(x, y)| x + y).collect()
}

fn short(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: does_the_group_delay_mark_the_seam <file>...");
        std::process::exit(2);
    }

    let periods = default_periods(32);
    let r = DEFAULT_R;
    let radius = separation(r);
    // The bank's first memories fill from a zero state, and its changes there
    // describe that state rather than the stream.
    let warm = (3.0 / (1.0 - r)) as usize;

    println!("modal-offset share by role: the share of supertokens whose nearest cut");
    println!("is at the offset most common for their role. Higher means the grain");
    println!("lands at one place per construct. Every set holds the same K cuts.");
    println!("r {r}, cuts at least {radius} tokens apart, first {warm} tokens skipped");
    println!();
    println!(
        "{:<22} {:>10} {:>8} {:>7}  {:>7} {:>7} {:>7} {:>7}",
        "input", "sig toks", "units", "K", "seam", "gd", "power", "random"
    );

    // The second question's rows, printed after the first table so each table
    // reads as one comparison.
    let mut second: Vec<String> = Vec::new();
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
        let sig: Vec<&trex::token::Token> = toks.iter().filter(|t| t.is_significant()).collect();
        // Kind codes packed to the ones present, since the bank's state and
        // its cost a push are both the alphabet times the band count.
        let raw: Vec<u32> = sig.iter().map(|t| t.kind.code()).collect();
        let mut distinct = raw.clone();
        distinct.sort_unstable();
        distinct.dedup();
        let symbols: Vec<u32> =
            raw.iter().map(|s| distinct.partition_point(|d| d < s) as u32).collect();
        let units = trex::supertoken::supertokens_from(&toks, &bytes);

        let t0 = Instant::now();
        let seam = trex::seam::analyze_symbols(&symbols, &SeamConfig::default());
        eprintln!("  seam over {} symbols in {:.1} s", symbols.len(), t0.elapsed().as_secs_f64());

        let t1 = Instant::now();
        let mut bank = Bank::new(distinct.len(), &periods, r);
        let mut gd = vec![0.0f32; periods.len()];
        let mut pw = vec![0.0f32; periods.len()];
        let mut prev_gd = vec![0.0f32; periods.len()];
        let mut prev_lp = vec![0.0f32; periods.len()];
        let mut gd_change = vec![0.0f32; symbols.len()];
        let mut pw_change = vec![0.0f32; symbols.len()];
        for (t, &s) in symbols.iter().enumerate() {
            bank.push(s);
            bank.read_into(&mut gd, &mut pw);
            let mut dg = 0.0f32;
            let mut dp = 0.0f32;
            // The power moves by orders of magnitude across a stream, so its
            // change is read on the log, where a doubling is one step wherever
            // it happens.
            for (((&g, &p), pg), pl) in
                gd.iter().zip(&pw).zip(prev_gd.iter_mut()).zip(prev_lp.iter_mut())
            {
                let lp = (p + f32::EPSILON).ln();
                if t > 0 {
                    dg += (g - *pg).abs();
                    dp += (lp - *pl).abs();
                }
                *pg = g;
                *pl = lp;
            }
            gd_change[t] = dg;
            pw_change[t] = dp;
        }
        for c in gd_change.iter_mut().take(warm) {
            *c = 0.0;
        }
        for c in pw_change.iter_mut().take(warm) {
            *c = 0.0;
        }
        eprintln!(
            "  bank over {} symbols, {} classes, {} periods in {:.1} s",
            symbols.len(),
            distinct.len(),
            periods.len(),
            t1.elapsed().as_secs_f64()
        );

        let gd_ranked = top_separated(&gd_change, radius);
        let pw_ranked = top_separated(&pw_change, radius);
        // The seam's cuts strongest first. A cut is the symbol a segment
        // starts at, and its boundary strength is indexed by that symbol.
        let mut seam_ranked: Vec<usize> =
            seam.cuts.iter().copied().filter(|&c| c > 0 && c < symbols.len()).collect();
        seam_ranked.sort_unstable_by(|&a, &b| seam.boundary[b].total_cmp(&seam.boundary[a]));

        let k = seam_ranked.len().min(gd_ranked.len()).min(pw_ranked.len());
        if k == 0 || units.is_empty() {
            println!(
                "{name:<22} {:>10} {:>8} {k:>7}  nothing to compare",
                symbols.len(),
                units.len()
            );
            continue;
        }

        let mut rng = 0x5EED_u64 ^ (symbols.len() as u64);
        let random: Vec<usize> =
            (0..k).map(|_| (splitmix(&mut rng) % symbols.len() as u64) as usize).collect();

        let to_bytes = |idx: &[usize]| -> Vec<usize> {
            let mut v: Vec<usize> = idx.iter().map(|&i| sig[i].start()).collect();
            v.sort_unstable();
            v.dedup();
            v
        };
        let seam_b = to_bytes(&seam_ranked[..k]);
        let gd_b = to_bytes(&gd_ranked[..k]);
        let pw_b = to_bytes(&pw_ranked[..k]);
        let rnd_b = to_bytes(&random);

        // One call scores three sets through the context's regime, shape and
        // seam slots. The slot names are the context's own; which set went in
        // which slot is what the columns say.
        let share = |a: &[usize], b: &[usize], c: &[usize]| {
            ContextField {
                at_token: Vec::new(),
                of_unit: Vec::new(),
                at_unit: Vec::new(),
                agreement: agreement(&units, &toks, a, b, c),
            }
            .alignment()
        };
        let (s_seam, s_gd, s_pw) = share(&seam_b, &gd_b, &pw_b);
        let (s_rnd, _, _) = share(&rnd_b, &rnd_b, &rnd_b);

        println!(
            "{name:<22} {:>10} {:>8} {k:>7}  {s_seam:>7.3} {s_gd:>7.3} {s_pw:>7.3} {s_rnd:>7.3}",
            symbols.len(),
            units.len()
        );

        // Whether the group delay adds to what is already read, which is a
        // different question from whether it is stronger on its own: two weak
        // signals can each carry what the other lacks. Every arm is selected by
        // the same rule at the same K, so two arms differ only in their signal.
        // The shifted arm moves the group delay half the stream along, keeping
        // its values and its local smoothness and losing only where its changes
        // land - so a gain over it is a gain from position, not from summing in
        // a second signal of any kind.
        let n_sym = symbols.len();
        assert_eq!(seam.boundary.len(), n_sym, "one seam strength a symbol");
        let half = n_sym / 2;
        let gd_shift: Vec<f32> = (0..n_sym).map(|i| gd_change[(i + half) % n_sym]).collect();
        let rk_pw = ranks(&pw_change);
        let rk_gd = ranks(&gd_change);
        let rk_gds = ranks(&gd_shift);
        let rk_seam = ranks(&seam.boundary);
        let rho = spearman(&rk_gd, &rk_pw, warm);

        let arm = |score: &[f32]| -> Vec<usize> {
            let mut picked = top_separated(score, radius);
            picked.truncate(k);
            to_bytes(&picked)
        };
        let (a_pw, a_pwgd, a_pwgds) =
            share(&arm(&rk_pw), &arm(&plus(&rk_pw, &rk_gd)), &arm(&plus(&rk_pw, &rk_gds)));
        let (a_s, a_sgd, a_sgds) = share(
            &arm(&rk_seam),
            &arm(&plus(&rk_seam, &rk_gd)),
            &arm(&plus(&rk_seam, &rk_gds)),
        );
        second.push(format!(
            "{name:<22} {rho:>6.3}  {a_pw:>7.3} {a_pwgd:>7.3} {a_pwgds:>7.3}  \
             {a_s:>7.3} {a_sgd:>7.3} {a_sgds:>7.3}"
        ));
    }

    println!();
    println!("does the group delay add to what is already read? rho is the rank correlation");
    println!("of its changes with the power's. Every arm holds the same K cuts, taken by one");
    println!("rule; ~ is the group delay moved half the stream along, which keeps its values");
    println!("and loses only where they land.");
    println!();
    println!(
        "{:<22} {:>6}  {:>7} {:>7} {:>7}  {:>7} {:>7} {:>7}",
        "input", "rho", "pw", "pw+gd", "pw+gd~", "seam", "seam+gd", "seam+gd~"
    );
    for row in &second {
        println!("{row}");
    }
    if unreadable > 0 {
        eprintln!("{unreadable} of {} inputs could not be read", args.len());
        std::process::exit(1);
    }
}
