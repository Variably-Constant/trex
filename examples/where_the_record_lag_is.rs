//! Which of a stream's real periodicities is its record, read on real files.
//!
//! [`trex::context::record_period`] returns the smallest lag reaching the
//! largest share, and its own note says why: a record's multiples repeat as
//! well as it does, so the smallest wins. That reasoning rejects multiples of
//! a record correctly and says nothing about a record's own parts, which
//! repeat as well as it does too.
//!
//! The consequence is measurable rather than theoretical. A clippy log reads
//! period 2 at a lift of 2.0 and is cut into 2,650,134 records, one per two
//! significant tokens, where its record is plainly its line. A table answers
//! six different lags across six slices of one file, every one of them a real
//! periodicity, where a table has one row width.
//!
//! So this prints the whole profile rather than the winner: every lag from 2
//! to the ceiling with its share, read against the chance rate that a share
//! only means something above. Beside it, the line as a token count - the
//! answer a log and a table already have - so it can be seen whether the
//! record's lag is inside the ceiling at all, and whether it clears.
//!
//! Each file is read as `trex scan` reads it (`trex::encoding::decode`), so a
//! UTF-16 log is profiled as the text it holds.
//!
//! Run: `cargo run --release --example where_the_record_lag_is -- <file>...`

use std::path::Path;

/// How many of the strongest lags to name.
const TOP: usize = 4;

/// The significant tokens on each line, as quantiles, which is the record
/// length a log and a table already know.
fn tokens_per_line(bytes: &[u8]) -> Option<(usize, usize, usize)> {
    let mut counts: Vec<usize> = Vec::new();
    for line in bytes.split(|&b| b == b'\n') {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let toks = trex::lexer::lex(line);
        counts.push(toks.iter().filter(|t| t.is_significant()).count());
    }
    if counts.is_empty() {
        return None;
    }
    counts.sort_unstable();
    let at = |q: f64| counts[((counts.len() - 1) as f64 * q) as usize];
    Some((at(0.25), at(0.5), at(0.75)))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: where_the_record_lag_is <file>...");
        std::process::exit(2);
    }

    // An input that cannot be read is a section missing from the report, and a
    // report missing a section reads as complete. So a run with one exits
    // failing.
    let mut unreadable = 0usize;
    for arg in &args {
        let path = Path::new(arg);
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let raw = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                println!("{name}: unreadable: {e}");
                unreadable += 1;
                continue;
            }
        };
        // Read as `trex scan` reads a file: a UTF-16 log profiled as its raw
        // bytes is a stream with every other byte zero, which no scan sees.
        let read = raw.len();
        let bytes = trex::encoding::decode(raw);
        let slice = &bytes[..];
        let toks = trex::lexer::lex(slice);
        let (chance, profile) = trex::context::record_period_profile(&toks, slice);

        if slice.len() == read {
            println!("== {name} ({} bytes read)", slice.len());
        } else {
            println!("== {name} ({} bytes as trex scan decodes the {read} read)", slice.len());
        }
        match tokens_per_line(slice) {
            Some((lo, mid, hi)) => println!(
                "   significant tokens per line: {lo} / {mid} / {hi} at the quartiles"
            ),
            None => println!("   no lines"),
        }
        println!("   chance rate {chance:.3}");

        if profile.is_empty() {
            println!("   no lag has two periods to compare");
            continue;
        }

        // By lift rather than share, since the share's baseline is the chance
        // rate and every lag is read against the same one here.
        let mut ranked = profile.clone();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let named: Vec<String> = ranked
            .iter()
            .take(TOP)
            .map(|(lag, share)| {
                let lift = if chance > 0.0 { f64::from(*share) / f64::from(chance) } else { 0.0 };
                format!("{lag}:{share:.2}({lift:.1}x)")
            })
            .collect();
        println!("   strongest lags  {}", named.join("  "));

        // What the shipped rule picks out of that profile, and the longest lag
        // that repeats as well - the two ends of the question.
        let kept = trex::context::record_period(&toks, slice);
        let floor = f64::from(chance) * f64::from(trex::context::PERIOD_LIFT_FLOOR);
        let longest = profile
            .iter()
            .rfind(|(_, s)| f64::from(*s) >= floor)
            .map(|(lag, s)| (*lag, *s));
        match (kept, longest) {
            (Some(k), Some((l, s))) => println!(
                "   shipped takes {k}; the longest lag clearing the gate is {l} at {s:.2}"
            ),
            (None, Some((l, s))) => {
                println!("   shipped takes none; the longest clearing the gate is {l} at {s:.2}");
            }
            (Some(k), None) => println!("   shipped takes {k}; no lag clears the gate"),
            (None, None) => println!("   shipped takes none, and no lag clears the gate"),
        }

        // The same rule one rung up. A record too long in tokens to be reached
        // may be a unit or two in supertokens, which is inside the search.
        let units = trex::supertoken::supertokens_from(&toks, slice);
        let roles: Vec<u32> = units.iter().map(|u| u.role.code()).collect();
        let (u_chance, u_profile) = trex::context::period_profile_of(&roles);
        let lines = slice.iter().filter(|&&b| b == b'\n').count().max(1);
        println!(
            "   supertokens: {} over {lines} lines, {:.2} per line, chance {u_chance:.3}",
            units.len(),
            units.len() as f64 / lines as f64
        );
        if u_profile.is_empty() {
            println!("   too few units to read a lag over");
        } else {
            let mut u_ranked = u_profile.clone();
            u_ranked
                .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            let named: Vec<String> = u_ranked
                .iter()
                .take(TOP)
                .map(|(lag, share)| {
                    let lift =
                        if u_chance > 0.0 { f64::from(*share) / f64::from(u_chance) } else { 0.0 };
                    format!("{lag}:{share:.2}({lift:.1}x)")
                })
                .collect();
            println!("   strongest unit lags  {}", named.join("  "));
        }

        // Six roles put the chance rate near 0.6, which is most of what a
        // share can reach, so a real periodicity has little room to rise
        // above it. Folding the unit's size in by its magnitude widens the
        // alphabet without keying on a length that varies run to run.
        let sized: Vec<u32> = units
            .iter()
            .map(|u| {
                let len = (u.end - u.start).max(1);
                u.role.code() * 8 + len.ilog2().min(7)
            })
            .collect();
        let (s_chance, s_profile) = trex::context::period_profile_of(&sized);
        if !s_profile.is_empty() {
            let mut s_ranked = s_profile;
            s_ranked
                .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            let named: Vec<String> = s_ranked
                .iter()
                .take(TOP)
                .map(|(lag, share)| {
                    let lift =
                        if s_chance > 0.0 { f64::from(*share) / f64::from(s_chance) } else { 0.0 };
                    format!("{lag}:{share:.2}({lift:.1}x)")
                })
                .collect();
            println!(
                "   with size folded in, chance {s_chance:.3}:  {}",
                named.join("  ")
            );
        }
        println!();
    }
    if unreadable > 0 {
        eprintln!("{unreadable} of {} inputs could not be read", args.len());
        std::process::exit(1);
    }
}
