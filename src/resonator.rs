//! A resonator bank: the filterbank with complex poles, so a band has a
//! frequency and not only a decay.
//!
//! [`crate::spectral`] runs `x[t] = a*x[t-1] + (1-a)*u[t]` with `a` real. That
//! is a diagonal linear recurrence whose poles are on the real axis, which
//! makes every band low-pass: it measures how fast something fades and cannot
//! measure how fast it repeats. Period is recovered there by a separate
//! bounded autocorrelation, with its own window, maximum lag and hop.
//!
//! Moving the pole off the real axis, `a = r * exp(i * theta)`, gives each
//! band a center frequency as well as a decay, so the bank becomes band-pass.
//! Period then falls out of the same causal single-pass recurrence the decay
//! bands already use, and the phase of the resonator gives the reader's
//! position in the cycle.
//!
//! The poles are prescribed, not learned: one resonator per period in a
//! declared list, with `theta = 2*pi/period`. The state-space literature that
//! motivates the parameterization trains its poles; nothing here does.
//!
//! ## Grain
//!
//! The bank consumes a stream of symbol codes and an alphabet size, so the
//! same bank reads bytes by class, tokens by kind, or supertokens by role.
//! That is a re-grounding rather than a fold: a token-kind stream is not a
//! function of the byte-class stream, because the lexer's boundaries are not
//! in it, so each grain is its own run rather than an aggregation of a finer
//! one.
//!
//! Why it matters which grain: a byte-grain period finds only fixed-width
//! structure, since a ragged table moves its delimiter offsets every row. A
//! token-grain period sees a field as one symbol whatever its width, so the
//! row period survives. [`crate::shape`] makes exactly this argument against
//! [`crate::spectral`] and answers it with a second axis; here it is the same
//! bank pointed at a different stream.
//!
//! ## How fine the alphabet is, and why the choice is the caller's
//!
//! Grain settles which stream is read; the alphabet settles how much of each
//! symbol survives. [`over_bytes`] reads the five classes
//! [`crate::spectral::classify`] assigns, which is the coarsest useful
//! partition of a byte, and it discards value outright: base64 of a
//! structured payload has a four-character quantum that no class boundary
//! marks, and the class reading is at the mean there while
//! [`over_byte_values`] reads three times it.
//!
//! Neither is the better alphabet. Classes generalize, so they find a shape
//! that recurs across different bytes; values discriminate, so they find a
//! period that lives in the bytes themselves. Which one is right is a
//! property of the question, so `analyze_symbols` takes the alphabet as a
//! parameter and the named entry points are conveniences over it rather than
//! the supported set.
//!
//! The cost of a fine alphabet is not what it appears. A resonator per
//! (symbol, period) suggests reading raw bytes costs fifty times what reading
//! classes costs, and the difference measures under one and a half times,
//! because only the symbol that fires is touched per position. What remains
//! is the periodic fold and the larger state's cache footprint. See
//! `analyze_symbols`.
//!
//! ## What the dominant period is, and is not
//!
//! Every resonator whose period divides a repeating unit responds to it, so a
//! stream with a primitive period carries power at that period and at its
//! harmonics. Which of them is loudest depends on where the classes are
//! inside the unit, not on which is primitive: a comma-separated fixed-width
//! row peaks at the field rather than the row, and a unit whose classes nearly
//! alternate peaks at two. [`Spectrum::dominant`] is therefore the loudest
//! band, not the record length. A caller wanting the record has to find the
//! fundamental that explains the peaks rather than take the peak, and
//! [`Spectrum::power_at`] is there to check a candidate directly.
//!
//! ## What the phase carries, and what the group delay carries
//!
//! Two readings come off the same states, and they are not equivalent.
//!
//! The frequency correction is degenerate for this bank, as algebra rather
//! than as a fact about any stream. Reassignment reads it as `w_hat = w +
//! Im{X_Dh * conj(X) / |X|^2}` with `h_D = dh/dn` (Fitz and Fulop, "A Unified
//! Theory of Time-Frequency Reassignment", arXiv 0903.3080, equation 65), and
//! the analysis window here is the one-sided exponential `h(n) = r^n`, which
//! is its own derivative up to a constant: `dh/dn = ln(r) * h(n)`. So `X_Dh`
//! is a real multiple of `X`, the product `X_Dh * conj(X)` is `ln(r)*|X|^2`,
//! its imaginary part is zero, and the correction returns `w` for every input.
//! Writing the state as `z[t] = exp(i*theta*t) * W[t]` says the same from the
//! other side: the phase advances at exactly `theta` per position whatever the
//! stream holds.
//!
//! Two consequences for the reported [`Spectrum::phase`]. It is not a
//! coordinate: on fixed-width input it is `position mod period` to a tenth of
//! a column, which a counter gives exactly and for nothing, and its offset is
//! set by where the classes are inside the unit rather than by where the
//! record starts - about seven columns into the row on a comma-separated
//! corpus. And the residual `arg W` is not a periodicity test, since `W` is an
//! infinite-impulse-response smoothing of a windowed transform coefficient, so
//! it drifts slowly for any input at all and how still it stays measures the
//! filter's own smoothing. Three degeneracies underlie any attempt to read it
//! and are recorded because each one looks like the bug: period two is
//! real-valued so its phase can only be zero or pi; samples closer together
//! than `1/(1-r)` share most of their input and agree trivially; and a sample
//! spacing commensurate with a period compares the state to itself one cycle
//! earlier.
//!
//! The group delay is the reading that does not degenerate. Equation 64 is
//! `t_hat = t - Re{X_Th * conj(X) / |X|^2}` with the ramped window
//! `h_T(n) = n * h(n)`, and `n * r^n` is not a multiple of `r^n`. Neither
//! equation reads the phase itself - both are ratios of two filter outputs, so
//! the class
//! switch that makes the reported phase jump does not reach them.
//! [`Spectrum::group_delay`] carries it, in positions: where inside the bank's
//! memory the band's energy is.
//!
//! It separates what the power does not. Over five real files at `r` of 0.98,
//! the five-class alphabet and 40000 bytes, a tab-separated corpus carrying
//! five fields on every one of its rows at a mean field width of 29.8, with
//! ragged line lengths, reads a group delay of 0.76 at period 32 where prose,
//! code, a log and a mixed corpus read 10.3 to 19.8, and its spread is a third
//! of theirs. The power ranks that same corpus last of the five. The
//! separation is not a property of the file: at period 4 it matches the
//! others and differs from them only at the bands where its geometry is.
//!
//! ### Across grains
//!
//! The phase difference between two grains is a further reading, and it is
//! token density rather than geometry. Each grain counts in its own clock and
//! the ratio between the clocks is content rather than a constant, so the
//! difference is not a counter minus itself. Writing the difference as
//! `theta_b * pos - theta_k * ntok(pos) + const`, its rate of change is
//! `theta_b - theta_k * dntok/dpos`, which is the local token density, so it
//! holds still whenever tokens accrue at a steady average rate per byte - and
//! a ragged table satisfies that as readily as a fixed-width one, its row
//! lengths varying around a stable mean. Sweeping the byte period against a
//! fixed token period, a corpus of twelve-byte rows locks hardest at fifteen
//! rather than twelve, and a ragged corpus with no fixed row width locks
//! harder than either fixed-width corpus.

/// One resonator's state: a complex accumulator.
#[derive(Clone, Copy, Debug, Default)]
struct C {
    re: f32,
    im: f32,
}

impl C {
    #[inline]
    fn mul(self, o: C) -> Self {
        C { re: self.re * o.re - self.im * o.im, im: self.re * o.im + self.im * o.re }
    }

    #[inline]
    fn add(self, o: C) -> Self {
        C { re: self.re + o.re, im: self.im + o.im }
    }

    #[inline]
    fn sub(self, o: C) -> Self {
        C { re: self.re - o.re, im: self.im - o.im }
    }

    /// Scaled by a real factor, which is what a position index is when it
    /// weights a fire for the ramped window.
    #[inline]
    fn scale(self, k: f32) -> Self {
        C { re: self.re * k, im: self.im * k }
    }

    /// `Re(self * conj(o))`, the numerator of the group-delay reading.
    #[inline]
    fn dot(self, o: C) -> f32 {
        self.re * o.re + self.im * o.im
    }

    /// `r * exp(i * theta)` scaled to a magnitude, the pole raised to a power.
    #[inline]
    fn polar(mag: f32, ang: f32) -> Self {
        C { re: mag * ang.cos(), im: mag * ang.sin() }
    }

    #[inline]
    fn power(self) -> f32 {
        self.re * self.re + self.im * self.im
    }

    #[inline]
    fn phase(self) -> f32 {
        self.im.atan2(self.re)
    }
}

/// The reading of a resonator bank over one stream.
#[derive(Clone, Debug, Default)]
pub struct Spectrum {
    /// The periods the bank was tuned to, ascending.
    pub periods: Vec<u16>,
    /// Power at each period, normalized so a longer memory does not simply
    /// read louder.
    pub power: Vec<f32>,
    /// Phase at each period, radians in `(-pi, pi]`: the position of the end
    /// of the stream in the cycle.
    pub phase: Vec<f32>,
    /// Group delay at each period, in positions: where inside the resonator's
    /// memory the band's energy is. Read from a ratio of two filter outputs
    /// rather than from [`Spectrum::phase`], so it is unaffected by the class
    /// switch that makes the reported phase jump.
    pub group_delay: Vec<f32>,
}

impl Spectrum {
    /// The period carrying the most power, or `None` when the bank is empty
    /// or found nothing.
    #[must_use]
    pub fn dominant(&self) -> Option<u16> {
        let (i, p) = self
            .power
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))?;
        (*p > 0.0).then(|| self.periods[i])
    }

    /// Power at one period, or `0.0` when the bank was not tuned to it.
    #[must_use]
    pub fn power_at(&self, period: u16) -> f32 {
        self.periods
            .iter()
            .position(|&p| p == period)
            .map_or(0.0, |i| self.power[i])
    }

    /// Phase at one period, or `0.0` when the bank was not tuned to it.
    #[must_use]
    pub fn phase_at(&self, period: u16) -> f32 {
        self.periods
            .iter()
            .position(|&p| p == period)
            .map_or(0.0, |i| self.phase[i])
    }

    /// Group delay at one period, or `0.0` when the bank was not tuned to it.
    #[must_use]
    pub fn group_delay_at(&self, period: u16) -> f32 {
        self.periods
            .iter()
            .position(|&p| p == period)
            .map_or(0.0, |i| self.group_delay[i])
    }

}

/// Pole radius. Sets how many cycles the bank integrates over: the effective
/// memory is about `1 / (1 - r)` samples, so 0.98 holds roughly fifty.
pub const DEFAULT_R: f32 = 0.98;

/// The periods a bank is tuned to by default: every period from 2 up, which
/// covers a CSV row, a fixed-width record and a base64 quantum without
/// choosing between them.
#[must_use]
pub fn default_periods(max: u16) -> Vec<u16> {
    (2..=max.max(2)).collect()
}

/// Positions between folds of the de-rotated accumulator.
///
/// Within a block the de-rotating phasor grows as `r^-v`, so the block length
/// is the dynamic range one block spans. Holding that near a hundred keeps the
/// earliest and latest contributions inside a block comparable in `f32`, which
/// a much longer block would not: at `r` of 0.98 a block of a thousand spans a
/// factor of `1e9` and the early fires would be lost in rounding.
fn block_len(r: f32) -> usize {
    let decay = -r.clamp(f32::EPSILON, 1.0 - f32::EPSILON).ln();
    ((4.6 / decay) as usize).clamp(16, 4096)
}

/// Run a resonator bank over a stream of symbol codes.
///
/// One resonator per (symbol class, period): the bank detects that the class
/// sequence repeats with a period, which is what a row, a record or a quantum
/// is. Power at a period is summed over classes, so a pattern spread across
/// several classes reads as one periodicity rather than several weak ones.
///
/// `alphabet` bounds the symbol codes; a code at or past it is ignored rather
/// than folded, so an unexpected class cannot alias onto a real one.
///
/// The codes must be DENSE. State is `alphabet * periods` complex values and
/// the periodic fold touches every one of them, so the alphabet is a cost and
/// not just a bound: it sets the allocation and the work done at each block
/// boundary. A caller holding sparse codes - [`crate::shape::shape_class`] is
/// `kind.code() << 16` plus a silhouette, spread over hundreds of thousands -
/// packs them to the distinct values present first. Sizing the alphabet from
/// the largest code instead is what turns a linear pass into one folding
/// millions of values every few hundred symbols.
#[must_use]
pub fn analyze_symbols(symbols: &[u32], alphabet: usize, periods: &[u16], r: f32) -> Spectrum {
    let np = periods.len();
    if np == 0 || alphabet == 0 {
        return Spectrum::default();
    }
    let thetas: Vec<f32> = periods
        .iter()
        .map(|&p| std::f32::consts::TAU / f32::from(p.max(1)))
        .collect();
    // The pole and its inverse, per period.
    let w: Vec<C> = thetas.iter().map(|&t| C::polar(r, t)).collect();
    let winv: Vec<C> = thetas.iter().map(|&t| C::polar(1.0 / r, -t)).collect();

    // The excitation is zero-mean. A raw class indicator is mostly constant,
    // and a resonator tuned to a long period is near DC, so it would
    // accumulate that constant and read as the strongest band whatever the
    // stream contained. Subtracting the expected rate leaves only the part
    // that actually varies with position.
    let dc = 1.0 / alphabet as f32;

    // Splitting the excitation is what makes the cost independent of the
    // alphabet. Writing the indicator as `[s == c] - dc`, the `-dc` half is
    // the same for every class, so it is one shared accumulator rather than
    // one per class, and the `[s == c]` half is non-zero only for the class
    // that actually fired. A direct implementation updates every class at
    // every position, which is O(alphabet * periods) per symbol and rules out
    // reading raw byte values; this is O(periods).
    //
    // Only the firing class is touched because the accumulator is held
    // de-rotated: `acc` carries `sum(w^-v)` over the fires so far rather than
    // the rotated sum, so positions that did not fire need no work. The price
    // is that the de-rotating phasor grows as `r^-v`, which is why it is
    // folded back every `block` positions.
    let block = block_len(r);
    let wblock: Vec<C> = thetas.iter().map(|&t| C::polar(r.powi(block as i32), t * block as f32)).collect();

    // The state is a pair per period rather than a real array and an
    // imaginary one: the split form was measured at 1.8x the time of this
    // one on both the byte and the token stream (`benches/axis_grain`), the
    // eight separate arrays costing the loop more in alias checks than the
    // pair costs it in shuffles.
    let mut acc = vec![C::default(); alphabet * np];
    let mut phasor = vec![C { re: 1.0, im: 0.0 }; np];
    let mut shared = vec![C::default(); np];
    let mut off = 0usize;

    // The ramped window, for the group delay. Cascading a second pole,
    // `z2[t] = a*z2[t-1] + z[t]`, sums to `sum (m+1) a^m x[t-m]`, which is the
    // window `n*h(n)` reassignment asks for. Held de-rotated like `acc`: with
    // the block-local offset of a fire written `o`, `acc` carries `sum w^-o`
    // and `ramp` carries `sum o*w^-o`, and the state at offset `v` is
    // `w^v * ((v+1)*acc - ramp)`. Only the firing class is touched, so the
    // second window costs what the first does.
    let mut ramp = vec![C::default(); alphabet * np];
    // The `-dc` half cascades the same way, and is shared across classes
    // because it is the same at every position.
    let mut shared2 = vec![C::default(); np];

    let mut advanced = 0usize;
    for &s in symbols {
        let c = s as usize;
        // A code at or past the alphabet advances nothing, so it neither
        // aliases onto a real class nor decays the state around it, and it
        // is not counted in the stream's length.
        if c >= alphabet {
            continue;
        }
        advanced += 1;
        if off == block {
            // Every class folds by the same power of the pole, so this pass
            // is the only work proportional to the alphabet, and it runs once
            // per block rather than once per position.
            for cls in 0..alphabet {
                let base = cls * np;
                for k in 0..np {
                    // Moving the block start forward by `block` takes every
                    // recorded offset `o` to `o - block`, so the ramped sum
                    // loses `block` copies of the plain one. It is folded
                    // first because it reads the unfolded `acc`.
                    let flat = acc[base + k];
                    ramp[base + k] =
                        wblock[k].mul(ramp[base + k].sub(flat.scale(block as f32)));
                    acc[base + k] = flat.mul(wblock[k]);
                }
            }
            phasor.fill(C { re: 1.0, im: 0.0 });
            off = 0;
        }
        let base = c * np;
        let o = off as f32;
        for k in 0..np {
            shared[k] = shared[k].mul(w[k]).add(C { re: dc, im: 0.0 });
            shared2[k] = shared2[k].mul(w[k]).add(shared[k]);
            acc[base + k] = acc[base + k].add(phasor[k]);
            ramp[base + k] = ramp[base + k].add(phasor[k].scale(o));
            phasor[k] = phasor[k].mul(winv[k]);
        }
        off += 1;
    }

    // The accumulator is de-rotated by the current block offset, so rotating
    // it forward by that offset recovers the resonator state.
    let live = off.saturating_sub(1) as f32;
    let rot: Vec<C> = thetas
        .iter()
        .map(|&t| C::polar(r.powf(live), t * live))
        .collect();
    let mut state = vec![C::default(); alphabet * np];
    // The ramped window's state at the same position, from which the group
    // delay is a ratio. `sum (m+1) a^m x[t-m]` is the plain state plus the
    // ramped one, so the window `n*h(n)` reassignment asks for is their
    // difference, taken below.
    let mut state_ramp = vec![C::default(); alphabet * np];
    for cls in 0..alphabet {
        let base = cls * np;
        for k in 0..np {
            state[base + k] = acc[base + k].mul(rot[k]).sub(shared[k]);
            let ramped = acc[base + k].scale(live + 1.0).sub(ramp[base + k]);
            state_ramp[base + k] = ramped.mul(rot[k]).sub(shared2[k]);
        }
    }

    // Normalize by the steady-state gain of a resonator, 1/(1-r), so a longer
    // memory does not read as more power, and by the symbols that advanced
    // the bank so a longer stream does not either.
    let gain = (1.0 - r).max(f32::EPSILON);
    let n = (advanced.max(1)) as f32;
    let scale = (gain * gain) / n;

    let mut power = vec![0.0f32; np];
    let mut phase = vec![0.0f32; np];
    let mut group_delay = vec![0.0f32; np];
    for k in 0..np {
        let mut acc = 0.0f32;
        let mut strongest = C::default();
        let mut weighted = 0.0f32;
        for c in 0..alphabet {
            let z = state[c * np + k];
            let e = z.power();
            acc += e;
            if e > strongest.power() {
                strongest = z;
            }
            // Reassignment's group delay for this class is `Re(X_Th *
            // conj(X)) / |X|^2`, and weighting each class by its own energy
            // cancels that denominator, leaving the numerators summed over a
            // total energy. It reads no phase, so the class switch that makes
            // the reported phase jump does not reach it.
            weighted += state_ramp[c * np + k].sub(z).dot(z);
        }
        power[k] = acc * scale;
        // The phase of the class carrying most of this period's energy: the
        // classes share the period, so an average over them would cancel.
        phase[k] = strongest.phase();
        // Weighted by each class's energy rather than taken from the loudest,
        // so the reading is continuous where the loudest class changes.
        group_delay[k] = if acc > f32::EPSILON { weighted / acc } else { 0.0 };
    }
    Spectrum { periods: periods.to_vec(), power, phase, group_delay }
}

/// Read the bank over the byte stream, by byte class, the bytes read as
/// UTF-8: a letter outside ASCII is the letter class every byte, any other
/// character outside ASCII its class on its first byte with the rest
/// advancing nothing, and a byte not part of a well-formed character is
/// class 4.
#[must_use]
pub fn over_bytes(bytes: &[u8], max_period: u16) -> Spectrum {
    let symbols = crate::spectral::class_symbols(bytes, crate::spectral::classify);
    analyze_symbols(&symbols, crate::spectral::N_CLASSES, &default_periods(max_period), DEFAULT_R)
}

/// Read the bank over the byte stream, by raw byte value.
///
/// The finest alphabet available over bytes, and it sees structure the
/// five-class reading discards. Base64 of a structured payload is the case:
/// its four-character quantum is a semantic grouping that no class boundary
/// marks, so [`over_bytes`] reads below-average power at period four, while
/// this reading puts it well above. What carries the distinction is that the
/// first character of a quantum is the top six bits of a byte and the last is
/// the low six, so the two draw on different parts of the payload's range -
/// a difference in value, which classes throw away.
///
/// It has nothing to find when the payload is incompressible: base64 of random
/// bytes is uniform over the alphabet at every position within the quantum, so
/// no reading of any fineness can locate the boundary.
#[must_use]
pub fn over_byte_values(bytes: &[u8], max_period: u16) -> Spectrum {
    let symbols: Vec<u32> = bytes.iter().map(|&b| u32::from(b)).collect();
    analyze_symbols(&symbols, 256, &default_periods(max_period), DEFAULT_R)
}

/// Read the bank over the token stream, by token kind.
///
/// Whitespace is dropped, as every token-grain reading drops it, so a run of
/// spaces does not put a beat in the period.
#[must_use]
pub fn over_tokens(toks: &[crate::token::Token], max_period: u16) -> Spectrum {
    let symbols: Vec<u32> = toks
        .iter()
        .filter(|t| t.is_significant())
        .map(|t| t.kind.code())
        .collect();
    // Codes run past the built-in block for user shapes, so the alphabet is
    // sized from what is present rather than from the built-in count.
    let alphabet = symbols.iter().copied().max().map_or(1, |m| m as usize + 1);
    analyze_symbols(&symbols, alphabet, &default_periods(max_period), DEFAULT_R)
}

/// Read the bank over the supertoken stream, by structural role.
#[must_use]
pub fn over_supertokens(units: &[crate::supertoken::SuperToken], max_period: u16) -> Spectrum {
    let symbols: Vec<u32> = units.iter().map(|u| u.role.code()).collect();
    let alphabet = symbols.iter().copied().max().map_or(1, |m| m as usize + 1);
    analyze_symbols(&symbols, alphabet, &default_periods(max_period), DEFAULT_R)
}

/// A resonator bank read at every position rather than once at the end.
///
/// [`analyze_symbols`] reads a whole stream once, and holds its accumulator
/// de-rotated so that a position touches only the class that fired, which is
/// what makes a whole-stream reading cost `O(periods)` a symbol. A reading
/// taken at every position needs every class's state at every position, and
/// that is the direct recurrence: each class's resonator decays and takes its
/// share of the excitation, so a push costs `O(alphabet * periods)`.
///
/// Read at the end of a stream it gives what [`analyze_symbols`] gives there,
/// and a test holds the two to each other - they are different algorithms
/// arriving at one quantity, so their agreement says something about both.
#[derive(Clone, Debug)]
pub struct Bank {
    alphabet: usize,
    poles: Vec<C>,
    dc: f32,
    /// Per (class, period), the resonator's state.
    z: Vec<C>,
    /// Per (class, period), the second pole in cascade, which is the ramped
    /// window the group delay divides by the plain one.
    z2: Vec<C>,
}

impl Bank {
    /// A bank over codes below `alphabet`, one resonator per period.
    #[must_use]
    pub fn new(alphabet: usize, periods: &[u16], r: f32) -> Self {
        let poles: Vec<C> = periods
            .iter()
            .map(|&p| C::polar(r, std::f32::consts::TAU / f32::from(p.max(1))))
            .collect();
        let states = alphabet * poles.len();
        Bank {
            alphabet,
            dc: if alphabet == 0 { 0.0 } else { 1.0 / alphabet as f32 },
            z: vec![C::default(); states],
            z2: vec![C::default(); states],
            poles,
        }
    }

    /// Take one symbol. A code at or past the alphabet advances nothing, as
    /// [`analyze_symbols`] lets it advance nothing.
    pub fn push(&mut self, symbol: u32) {
        let s = symbol as usize;
        if s >= self.alphabet {
            return;
        }
        let np = self.poles.len();
        for cls in 0..self.alphabet {
            let x = C { re: if cls == s { 1.0 - self.dc } else { -self.dc }, im: 0.0 };
            let base = cls * np;
            for (k, &pole) in self.poles.iter().enumerate() {
                let i = base + k;
                self.z[i] = self.z[i].mul(pole).add(x);
                self.z2[i] = self.z2[i].mul(pole).add(self.z[i]);
            }
        }
    }

    /// The group delay and the power at every period, as the bank is now.
    ///
    /// The power is the summed squared magnitude with no normalization, since
    /// a reading taken at every position is compared with its neighbors and
    /// not across streams.
    ///
    /// # Panics
    ///
    /// When either slice is not one entry a period long, which is a caller
    /// holding a buffer sized for another bank.
    pub fn read_into(&self, group_delay: &mut [f32], power: &mut [f32]) {
        let np = self.poles.len();
        assert_eq!(group_delay.len(), np, "one group delay a period");
        assert_eq!(power.len(), np, "one power a period");
        for (k, (gd, pw)) in group_delay.iter_mut().zip(power.iter_mut()).enumerate() {
            let mut num = 0.0f32;
            let mut den = 0.0f32;
            for c in 0..self.alphabet {
                let i = c * np + k;
                num += self.z2[i].sub(self.z[i]).dot(self.z[i]);
                den += self.z[i].power();
            }
            *pw = den;
            *gd = if den > f32::EPSILON { num / den } else { 0.0 };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The definition, written directly: every class updated at every
    /// position. This is what `analyze_symbols` computes, at
    /// `O(alphabet * periods)` per symbol instead of `O(periods)`.
    fn reference(symbols: &[u32], alphabet: usize, periods: &[u16], r: f32) -> Vec<f32> {
        let np = periods.len();
        let dc = 1.0 / alphabet as f32;
        let mut st = vec![C::default(); alphabet * np];
        for &s in symbols {
            let c = s as usize;
            if c >= alphabet {
                continue;
            }
            for cls in 0..alphabet {
                let x = if cls == c { 1.0 - dc } else { -dc };
                for k in 0..np {
                    let theta = std::f32::consts::TAU / f32::from(periods[k].max(1));
                    let z = st[cls * np + k];
                    st[cls * np + k] = C {
                        re: r * (z.re * theta.cos() - z.im * theta.sin()) + x,
                        im: r * (z.re * theta.sin() + z.im * theta.cos()),
                    };
                }
            }
        }
        let gain = (1.0 - r).max(f32::EPSILON);
        let advanced = symbols.iter().filter(|&&s| (s as usize) < alphabet).count();
        let scale = (gain * gain) / (advanced.max(1)) as f32;
        (0..np)
            .map(|k| (0..alphabet).map(|c| st[c * np + k].power()).sum::<f32>() * scale)
            .collect()
    }

    fn b64(data: &[u8]) -> Vec<u8> {
        const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = Vec::new();
        for ch in data.chunks(3) {
            let b = [ch[0], *ch.get(1).unwrap_or(&0), *ch.get(2).unwrap_or(&0)];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            for k in 0..4 {
                out.push(A[((n >> (18 - 6 * k)) & 63) as usize]);
            }
        }
        out
    }

    /// Power at a period against the bank's mean, so readings taken over
    /// different alphabets are comparable.
    fn peak_ratio(sp: &Spectrum, period: u16) -> f32 {
        let mean = sp.power.iter().sum::<f32>() / sp.power.len() as f32;
        if mean > 0.0 { sp.power_at(period) / mean } else { 0.0 }
    }

    /// A fixed-width table: every field the same width, so the row period is
    /// the same in bytes and in tokens.
    fn fixed_width(rows: usize) -> Vec<u8> {
        let mut s = String::new();
        for i in 0..rows {
            s.push_str(&format!("{:03},{:03},{:03}\n", i % 1000, (i * 7) % 1000, (i * 13) % 1000));
        }
        s.into_bytes()
    }

    /// A ragged table: the same three fields per row, but the numbers vary in
    /// width, so the byte offsets of the delimiters move every row while the
    /// token sequence stays identical.
    fn ragged(rows: usize) -> Vec<u8> {
        let mut s = String::new();
        for i in 0..rows {
            s.push_str(&format!("{},{},{}\n", i, i * 7919, (i * 13) % 100));
        }
        s.into_bytes()
    }

    #[test]
    fn the_bank_reports_the_finest_repeating_unit_not_the_record() {
        // "000,000,000\n" is twelve bytes, but its class sequence is
        // DDDP DDDP DDDP W: the four-byte field repeats three times per row,
        // so it carries more energy than the row does. The bank finds the
        // finest period that actually repeats, and a caller wanting the
        // record has to look past it - the row is a multiple of the field,
        // and shows as power at that multiple rather than as the peak.
        let sp = over_bytes(&fixed_width(200), 32);
        assert_eq!(sp.dominant(), Some(4), "the field is four bytes and repeats most often");
        assert!(sp.power_at(12) > 0.0, "the row period carries power as a multiple of it");
        assert!(
            sp.power_at(4) > sp.power_at(12),
            "and less than the field: {} vs {}",
            sp.power_at(12),
            sp.power_at(4)
        );
    }

    #[test]
    fn the_token_grain_finds_the_row_period_a_ragged_table_hides_from_bytes() {
        // Each row is Number , Number , Number = five significant tokens,
        // whatever the digits, so the token period is five regardless of
        // width. The byte period is not stable, because the delimiters move.
        let input = ragged(200);
        let toks = crate::lexer::lex(&input);
        let tok_sp = over_tokens(&toks, 32);
        assert_eq!(tok_sp.dominant(), Some(5), "five tokens per row, width-independent");

        // The byte reading does not find a stable row period on the same
        // input, which is the argument shape.rs makes against spectral.
        let byte_sp = over_bytes(&input, 32);
        assert_ne!(byte_sp.dominant(), Some(5), "bytes cannot see the token period");
    }

    /// The group delay written directly from its definition: cascade a second
    /// pole at every position for every class, with no de-rotation and no
    /// block fold. This is what the folded accumulator has to reproduce.
    fn reference_group_delay(
        symbols: &[u32],
        alphabet: usize,
        periods: &[u16],
        r: f32,
    ) -> Vec<f32> {
        let np = periods.len();
        let dc = 1.0 / alphabet as f32;
        let mut z = vec![C::default(); alphabet * np];
        let mut z2 = vec![C::default(); alphabet * np];
        for &s in symbols {
            let c = s as usize;
            if c >= alphabet {
                continue;
            }
            for cls in 0..alphabet {
                let x = if cls == c { 1.0 - dc } else { -dc };
                for (k, &period) in periods.iter().enumerate() {
                    let theta = std::f32::consts::TAU / f32::from(period.max(1));
                    let a = C::polar(r, theta);
                    let i = cls * np + k;
                    z[i] = z[i].mul(a).add(C { re: x, im: 0.0 });
                    z2[i] = z2[i].mul(a).add(z[i]);
                }
            }
        }
        (0..np)
            .map(|k| {
                let mut num = 0.0f32;
                let mut den = 0.0f32;
                for c in 0..alphabet {
                    let i = c * np + k;
                    num += z2[i].sub(z[i]).dot(z[i]);
                    den += z[i].power();
                }
                if den > f32::EPSILON { num / den } else { 0.0 }
            })
            .collect()
    }

    #[test]
    fn the_folded_ramp_reproduces_the_group_delay_definition() {
        // The ramped accumulator is held de-rotated and folded every
        // `block_len` positions, which is where a sign or an off-by-one in the
        // fold would hide. The input runs well past one block so the fold is
        // exercised rather than skipped.
        let periods = [3u16, 4, 5, 8, 12];
        let r = 0.9;
        let input = fixed_width(120);
        let symbols: Vec<u32> =
            input.iter().map(|&b| crate::spectral::classify(b) as u32).collect();
        assert!(
            symbols.len() > 3 * block_len(r),
            "input of {} must span several blocks of {}",
            symbols.len(),
            block_len(r)
        );

        let got = analyze_symbols(&symbols, crate::spectral::N_CLASSES, &periods, r);
        let want = reference_group_delay(&symbols, crate::spectral::N_CLASSES, &periods, r);
        for (k, &p) in periods.iter().enumerate() {
            let tol = 0.02 * want[k].abs().max(1.0);
            assert!(
                (got.group_delay[k] - want[k]).abs() <= tol,
                "period {p}: folded {} against direct {}",
                got.group_delay[k],
                want[k]
            );
        }
    }

    /// A bank pushed through a stream and read at its end gives the whole-
    /// stream reading. The two are different algorithms - the bank runs the
    /// recurrence directly, the fold holds it de-rotated and touches only the
    /// class that fired - so their agreement checks each against the other.
    #[test]
    fn a_bank_read_at_the_end_is_the_whole_stream_reading() {
        let periods = [3u16, 4, 5, 8, 12];
        let r = 0.9;
        let input = fixed_width(120);
        let symbols: Vec<u32> =
            input.iter().map(|&b| crate::spectral::classify(b) as u32).collect();

        let mut bank = Bank::new(crate::spectral::N_CLASSES, &periods, r);
        for &s in &symbols {
            bank.push(s);
        }
        let mut gd = vec![0.0f32; periods.len()];
        let mut pw = vec![0.0f32; periods.len()];
        bank.read_into(&mut gd, &mut pw);

        let whole = analyze_symbols(&symbols, crate::spectral::N_CLASSES, &periods, r);
        // The fold normalizes by the steady-state gain and the stream length;
        // the bank leaves its power raw, so it is scaled the same way here.
        let scale = (1.0 - r) * (1.0 - r) / symbols.len() as f32;
        for (k, &p) in periods.iter().enumerate() {
            let gd_tol = 0.02 * whole.group_delay[k].abs().max(1.0);
            assert!(
                (gd[k] - whole.group_delay[k]).abs() <= gd_tol,
                "period {p}: bank group delay {} against the fold's {}",
                gd[k],
                whole.group_delay[k]
            );
            let pw_tol = 0.02 * whole.power[k].abs().max(f32::EPSILON);
            assert!(
                (pw[k] * scale - whole.power[k]).abs() <= pw_tol,
                "period {p}: bank power {} against the fold's {}",
                pw[k] * scale,
                whole.power[k]
            );
        }
    }

    /// The bands a twelve-byte row excites, which is why the loudest one is
    /// not the record.
    ///
    /// "000,000,000\n" has the class sequence DDDP DDDP DDDP W. A unit of
    /// period twelve excites every band whose period divides twelve, so the
    /// power is at 4, 2, 12, 6 and 3 - the divisors - and the peak is the
    /// four-byte field. Reading the row back out of that set is the harmonic
    /// problem, and the reading that answers it is a chance-gated
    /// autocorrelation at the supertoken grain rather than anything the bank
    /// can do alone.
    #[test]
    fn every_band_dividing_the_row_carries_power_and_the_peak_is_the_field() {
        let sp = over_bytes(&fixed_width(200), 32);
        assert_eq!(sp.dominant(), Some(4), "the loudest band is the field");
        for divisor in [2u16, 3, 4, 6, 12] {
            assert!(sp.power_at(divisor) > 0.0, "the row excites period {divisor}");
        }
        // A period that does not divide twelve is not a harmonic of the row,
        // and reads well below every one that does.
        let least_divisor =
            [2u16, 3, 4, 6, 12].iter().map(|&d| sp.power_at(d)).fold(f32::MAX, f32::min);
        for stranger in [5u16, 7, 11] {
            assert!(
                sp.power_at(stranger) < least_divisor,
                "period {stranger} divides nothing and must read under the divisors"
            );
        }
    }

    #[test]
    fn phase_advances_through_the_cycle() {
        // Reading successively longer prefixes of a periodic stream moves the
        // phase; reading a whole number of cycles returns it near where it was.
        let full = fixed_width(20);
        let a = over_bytes(&full[..120], 32).phase_at(12);
        let b = over_bytes(&full[..126], 32).phase_at(12);
        let c = over_bytes(&full[..132], 32).phase_at(12);
        assert!((a - b).abs() > 0.1, "half a row apart, the phase differs");
        let wrapped = (a - c).abs().min(std::f32::consts::TAU - (a - c).abs());
        assert!(wrapped < 0.1, "a whole row apart, the phase returns: {a} vs {c}");
    }

    #[test]
    fn an_aperiodic_stream_has_no_dominant_peak() {
        // Prose has no fixed row, so no period should dominate sharply.
        let prose = b"the quick brown fox jumps over the lazy dog and then the dog looks up";
        let sp = over_bytes(prose, 32);
        let max = sp.power.iter().copied().fold(0.0f32, f32::max);
        let mean = sp.power.iter().sum::<f32>() / sp.power.len() as f32;
        assert!(max < mean * 6.0, "no sharp peak in aperiodic text: {max} vs mean {mean}");
    }

    #[test]
    fn the_supertoken_grain_reads_a_repeating_block() {
        // Three identical two-statement blocks: the role sequence repeats.
        let src = "a = 1;\nb = 2;\nc = 3;\nd = 4;\ne = 5;\nf = 6;\n";
        let toks = crate::lexer::lex(src.as_bytes());
        let units = crate::supertoken::supertokens_from(&toks, src.as_bytes());
        let sp = over_supertokens(&units, 8);
        assert!(!sp.periods.is_empty());
        assert!(sp.power.iter().any(|&p| p > 0.0), "the role stream carries power");
    }

    #[test]
    fn a_mark_outside_ascii_reads_as_its_ascii_counterpart() {
        // A mark's first byte carries its class and its other bytes advance
        // nothing, so the typographic line is the ASCII line to the bit.
        let typographic = over_bytes("it\u{2019}s a\u{2014}test \u{201C}here\u{201D} and \u{201C}there\u{201D}".as_bytes(), 16);
        let ascii = over_bytes(b"it's a-test \"here\" and \"there\"", 16);
        assert_eq!(typographic.power, ascii.power);
        assert_eq!(typographic.group_delay, ascii.group_delay);
        // A letter outside ASCII is a letter, so a Cyrillic word holds no
        // other class.
        let symbols = crate::spectral::class_symbols("слово".as_bytes(), crate::spectral::classify);
        assert_eq!(symbols, vec![1; "слово".len()]);
    }

    #[test]
    fn an_out_of_alphabet_symbol_is_ignored_rather_than_folded() {
        // A code at or past the alphabet must not alias onto a real class.
        let inside = analyze_symbols(&[0, 1, 0, 1, 0, 1], 2, &[2, 3], DEFAULT_R);
        let with_stray = analyze_symbols(&[0, 1, 0, 9, 1, 0, 1, 9], 2, &[2, 3], DEFAULT_R);
        assert!(with_stray.power_at(2) > 0.0);
        // A stray code is not in the stream at all: not a class, not a step,
        // not a unit of its length.
        assert_eq!(inside.power, with_stray.power);
        assert_eq!(inside.group_delay, with_stray.group_delay);
    }

    #[test]
    fn empty_and_degenerate_inputs_are_safe() {
        assert!(analyze_symbols(&[], 4, &[2, 3], DEFAULT_R).dominant().is_none());
        assert!(analyze_symbols(&[0, 1], 0, &[2], DEFAULT_R).periods.is_empty());
        assert!(analyze_symbols(&[0, 1], 4, &[], DEFAULT_R).periods.is_empty());
        assert_eq!(over_bytes(b"", 16).dominant(), None);
    }

    #[test]
    fn the_bank_agrees_with_the_autocorrelation_on_a_byte_period() {
        // Three letters then three digits is a square wave in the class
        // indicators, so its fundamental carries more than its harmonics and
        // both readings should land on six. The two routes there are
        // independent: spectral::dominant_period correlates raw byte values
        // across a bounded lag window, this bank integrates class indicators
        // through complex poles in a single causal pass.
        let src = "abc123".repeat(60).into_bytes();
        let (ac_period, ac_strength) = crate::spectral::dominant_period(&src, 32);
        assert_eq!(ac_period, 6, "autocorrelation finds the period");
        assert!(ac_strength > 0.0);
        assert_eq!(over_bytes(&src, 32).dominant(), Some(6), "and so does the bank");
    }

    #[test]
    fn a_harmonic_can_outweigh_the_fundamental() {
        // What "dominant" means here, stated as a test because it is easy to
        // read the bank as a record-length detector and be wrong. In "a1,bc2"
        // the class sequence has primitive period six, but the digits are at
        // positions one and five, which is nearly an alternation, so the
        // period-two harmonic carries twice the amplitude of the fundamental.
        // The bank reports the loudest period, not the primitive one; a caller
        // wanting the primitive period has to find the fundamental that
        // explains the harmonics rather than trusting the peak.
        let src = "a1,bc2".repeat(60).into_bytes();
        assert_eq!(over_bytes(&src, 32).dominant(), Some(2), "the harmonic is louder");
        assert!(over_bytes(&src, 32).power_at(6) > 0.0, "the fundamental is still present");

        // Byte-value autocorrelation weighs the same input differently and
        // does find six, so where a class harmonic dominates the two readings
        // answer different questions rather than one being broken.
        assert_eq!(crate::spectral::dominant_period(&src, 32).0, 6);
    }

    #[test]
    fn the_split_accumulator_matches_the_direct_definition() {
        // The stream is long enough to fold the de-rotated accumulator several
        // times, which is where a de-rotating scheme goes wrong if it goes
        // wrong at all.
        let periods = [3u16, 4, 7, 12, 31];
        let symbols: Vec<u32> =
            (0..4000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 27) % 5).collect();
        assert!(symbols.len() > 4 * block_len(DEFAULT_R), "several blocks are folded");

        let got = analyze_symbols(&symbols, 5, &periods, DEFAULT_R).power;
        let want = reference(&symbols, 5, &periods, DEFAULT_R);
        for (k, (&g, &w)) in got.iter().zip(want.iter()).enumerate() {
            let tol = w.abs() * 1e-2 + 1e-9;
            assert!((g - w).abs() <= tol, "period {}: {g} vs {w}", periods[k]);
        }
    }

    #[test]
    fn a_byte_value_alphabet_finds_the_base64_quantum_that_classes_discard() {
        // The four-character quantum is a semantic grouping, so no byte class
        // marks it and the five-class reading has nothing to lock onto. Raw
        // byte values keep the distinction that the first character of a
        // quantum draws on the high bits of a payload byte and the last on the
        // low bits, so the positions differ in value where they do not differ
        // in class.
        //
        // Read as a two-by-two: the elevation at period four needs both the
        // encoding and the fine alphabet, and every other cell is near the
        // mean. The corpus has to be long and varied for this to mean
        // anything - a short phrase cycled is strongly periodic in its own
        // right, and its plain form reads as high at period four as its
        // encoded form does, which says nothing about quanta.
        let prose: Vec<u8> = "it was the best of times it was the worst of times it was the age of wisdom it was the age of foolishness it was the epoch of belief "
            .bytes()
            .cycle()
            .take(3000)
            .collect();
        let enc = b64(&prose);

        let found = peak_ratio(&over_byte_values(&enc, 16), 4);
        assert!(found > 2.0, "byte values find the quantum: {found}");
        let by_class = peak_ratio(&over_bytes(&enc, 16), 4);
        assert!(by_class < 1.5, "the five classes do not: {by_class}");
        let unencoded = peak_ratio(&over_byte_values(&prose, 16), 4);
        assert!(unencoded < 1.5, "and there is no quantum before encoding: {unencoded}");

        // The remaining control. An incompressible payload is uniform over the
        // alphabet at every position within the quantum, so no reading of any
        // fineness can locate the boundary - the elevation above is a property
        // of structured payloads, not of the encoding alone.
        let noise: Vec<u8> =
            (0..3000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
        let random_payload = peak_ratio(&over_byte_values(&b64(&noise), 16), 4);
        assert!(random_payload < 1.5, "a random payload hides it: {random_payload}");
    }
}
