//! A byte-grain non-deterministic automaton, and the simulation that reads it.
//!
//! The single-pass engine's [`crate::nfa::Program`] consumes one significant
//! token an instruction, so determinizing it yields an automaton over token
//! kinds - and one over token kinds cannot reach the regex crate's throughput,
//! because TREX's lex alone already costs more than regex's whole run. An
//! automaton that can has to read bytes, and this is the representation it
//! reads.
//!
//! What a byte automaton must express that a regular expression cannot, and
//! which the lexer's recognizers need: trailing context (mail, address, MAC,
//! CIDR, the ISO date, the unit boundary), leading context (a quantity's sign,
//! which reads the byte before the run), and finite tables (the ITU country
//! codes). All three are native here - a table is states, and context either
//! side is a branch taken before or after the run rather than a construct the
//! language has to name.
//!
//! This module is the automaton and its simulation. Determinizing it is
//! separate and belongs beside the token determinizer.

use std::collections::HashMap;

/// Which bytes an edge accepts, as a 256-bit set.
///
/// A set rather than a range list: a class is tested once a byte in the hot
/// loop, and a bitmap answers in two instructions where a range list answers in
/// a loop over its ranges. Four words is the whole of it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ByteClass([u64; 4]);

impl ByteClass {
    /// The empty class, which accepts nothing.
    #[must_use]
    pub fn none() -> ByteClass {
        ByteClass([0; 4])
    }

    /// Every byte.
    #[must_use]
    pub fn any() -> ByteClass {
        ByteClass([u64::MAX; 4])
    }

    /// The class holding exactly `b`.
    #[must_use]
    pub fn just(b: u8) -> ByteClass {
        let mut c = ByteClass::none();
        c.add(b);
        c
    }

    /// The class holding `lo` through `hi`, both ends included.
    #[must_use]
    pub fn range(lo: u8, hi: u8) -> ByteClass {
        let mut c = ByteClass::none();
        for b in lo..=hi {
            c.add(b);
        }
        c
    }

    /// Put `b` in the class.
    pub fn add(&mut self, b: u8) {
        self.0[(b >> 6) as usize] |= 1u64 << (b & 63);
    }

    /// Every byte of `other` as well as this one's.
    #[must_use]
    pub fn union(mut self, other: ByteClass) -> ByteClass {
        for (a, b) in self.0.iter_mut().zip(other.0.iter()) {
            *a |= *b;
        }
        self
    }

    /// The bytes both classes hold.
    #[must_use]
    pub fn intersect(mut self, other: ByteClass) -> ByteClass {
        for (mine, theirs) in self.0.iter_mut().zip(other.0.iter()) {
            *mine &= *theirs;
        }
        self
    }

    /// Every byte this class does not hold.
    #[must_use]
    pub fn negate(mut self) -> ByteClass {
        for w in &mut self.0 {
            *w = !*w;
        }
        self
    }

    /// Whether `b` is in the class.
    #[must_use]
    pub fn has(&self, b: u8) -> bool {
        self.0[(b >> 6) as usize] & (1u64 << (b & 63)) != 0
    }

    /// Whether the class accepts nothing at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0 == [0; 4]
    }
}

/// One state of the automaton.
#[derive(Clone, Debug)]
pub enum State {
    /// Consume one byte the class holds, and go to `next`.
    Byte { class: ByteClass, next: usize },
    /// Go to both, the first preferred where a preference is read.
    Split(usize, usize),
    /// Accept, carrying which recognizer accepted.
    Accept(u32),
    /// Accept `id`, but only where the byte after the match is one `follow`
    /// holds, or the match reaches the end of the input and `at_end` admits an
    /// end there.
    ///
    /// The condition rides the acceptance rather than being a state of its own
    /// because every trailing context the lexer's recognizers carry is at the
    /// end of a run: a unit is a byte size only where no alphanumeric follows,
    /// so `10MB` is one and `10MBx` is a number beside a word. A rule applied
    /// afterward to the match the automaton chose cannot reach that reading -
    /// by then the longer one has won, and the shorter one it must fall back to
    /// was never carried.
    AcceptIf { id: u32, follow: ByteClass, at_end: bool },
}

/// A byte-grain automaton: its states, and where a run begins.
#[derive(Clone, Debug, Default)]
pub struct ByteNfa {
    states: Vec<State>,
    start: usize,
}

impl ByteNfa {
    /// The states, for a determinizer reading this as a subset construction.
    #[must_use]
    pub fn states(&self) -> &[State] {
        &self.states
    }

    /// Where a run begins.
    #[must_use]
    pub fn start(&self) -> usize {
        self.start
    }

    /// How many states it holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.states.len()
    }

    /// Whether it holds no state at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    /// The token `input` begins with, as the recognizer that owns it and the
    /// byte it ends at, or `None` where no recognizer claims one.
    ///
    /// Precedence decides before extent does: the lowest id that accepts
    /// anywhere wins, and only among acceptances carrying that id does the
    /// longest win. This is a lexer's rule rather than a regular expression's.
    /// `lexer::try_typed_token_chain` takes the first branch of its chain that
    /// matches at all and then that branch's own reach, which is why a forty-byte
    /// hash digest followed by `+` and more base64 bytes is a digest rather than
    /// the longer base64 blob beginning at the same place.
    ///
    /// Extent still decides within a recognizer, where `1` and `1.5` both begin
    /// a number and the token is the longer. The simulation carries a set of
    /// states a byte at a time and keeps the best pair it has seen, which is one
    /// pass over the prefix and no backtracking.
    #[must_use]
    pub fn recognize(&self, input: &[u8]) -> Option<(usize, u32)> {
        if self.states.is_empty() {
            return None;
        }
        let mut here: Vec<usize> = Vec::with_capacity(self.states.len());
        let mut next: Vec<usize> = Vec::with_capacity(self.states.len());
        let mut on: Vec<bool> = vec![false; self.states.len()];
        self.follow(self.start, &mut here, &mut on);
        let mut best = self.accepting(&here, 0, input.first().copied(), None);
        for (i, &b) in input.iter().enumerate() {
            next.clear();
            on.iter_mut().for_each(|m| *m = false);
            for &s in &here {
                if let State::Byte { class, next: to } = &self.states[s]
                    && class.has(b)
                {
                    self.follow(*to, &mut next, &mut on);
                }
            }
            std::mem::swap(&mut here, &mut next);
            if here.is_empty() {
                break;
            }
            best = self.accepting(&here, i + 1, input.get(i + 1).copied(), best);
        }
        best
    }

    /// Add `s` and everything reachable from it without reading a byte.
    ///
    /// Iterative rather than recursive, and marked: an automaton whose epsilon
    /// edges form a cycle - a star over something that can match nothing is
    /// one - would otherwise not terminate, and a deep one would nest as far
    /// as the pattern is long.
    fn follow(&self, s: usize, out: &mut Vec<usize>, on: &mut [bool]) {
        let mut stack = vec![s];
        while let Some(t) = stack.pop() {
            if on[t] {
                continue;
            }
            on[t] = true;
            match self.states[t] {
                State::Split(a, b) => {
                    stack.push(b);
                    stack.push(a);
                }
                State::Byte { .. } | State::Accept(_) | State::AcceptIf { .. } => out.push(t),
            }
        }
    }

    /// The best acceptance among `here` at byte `at`, against what is held.
    ///
    /// A lower recognizer id always wins, whatever the two reach, and a later
    /// end wins only between acceptances of one id. `after` is the byte the
    /// match would be followed by, which a conditional acceptance is read
    /// against and `None` where the match reaches the end of the input.
    fn accepting(
        &self,
        here: &[usize],
        at: usize,
        after: Option<u8>,
        held: Option<(usize, u32)>,
    ) -> Option<(usize, u32)> {
        let mut best = held;
        for &s in here {
            let Some(id) = self.accepts(s, after) else {
                continue;
            };
            best = match best {
                Some((_, who)) if who < id => best,
                Some((end, who)) if who == id && end >= at => best,
                _ => Some((at, id)),
            };
        }
        best
    }

    /// The automaton accepting what both of these accept, as recognizer `id`.
    ///
    /// A recognizer is often a shape and a condition over one run: a hash digest
    /// is a hex run of an exact length that must hold a hex letter somewhere, a
    /// base64 blob a run whose length is a multiple of four and which must hold
    /// a digit and both cases. A Thompson piece cannot say "somewhere in here",
    /// because it cannot share a suffix with another piece, so the shape and the
    /// condition are built apart and met here. Spelled as one piece instead, the
    /// digest's letter costs an alternation over every position the letter could
    /// take; met, it costs one small automaton against the shape.
    ///
    /// The product is over pairs of states and not over sets of them, so neither
    /// side is determinized and the result is an automaton like any other. A
    /// pair advances only on the bytes both its states admit and accepts only
    /// where both accept, which is also how the two trailing conditions meet: a
    /// byte has to satisfy both, and an end has to satisfy both.
    #[must_use]
    pub fn intersect(&self, other: &ByteNfa, id: u32) -> ByteNfa {
        let dead = State::Byte { class: ByteClass::none(), next: 0 };
        let mut states: Vec<State> = vec![dead.clone()];
        let mut index: HashMap<(usize, usize), usize> = HashMap::new();
        let mut queue: Vec<(usize, usize)> = Vec::new();
        let mut mine: Vec<usize> = Vec::new();
        let mut mine_on = vec![false; self.states.len()];
        let mut theirs: Vec<usize> = Vec::new();
        let mut theirs_on = vec![false; other.states.len()];

        let pair = |a: usize,
                        b: usize,
                        states: &mut Vec<State>,
                        index: &mut HashMap<(usize, usize), usize>,
                        queue: &mut Vec<(usize, usize)>| {
            *index.entry((a, b)).or_insert_with(|| {
                states.push(State::Byte { class: ByteClass::none(), next: 0 });
                queue.push((a, b));
                states.len() - 1
            })
        };

        self.follow(self.start, &mut mine, &mut mine_on);
        other.follow(other.start, &mut theirs, &mut theirs_on);
        let mut seeds: Vec<usize> = Vec::new();
        for &a in &mine {
            for &b in &theirs {
                seeds.push(pair(a, b, &mut states, &mut index, &mut queue));
            }
        }
        let start = fan(&seeds, 0, &mut states);

        let mut done = 0usize;
        while done < queue.len() {
            let (a, b) = queue[done];
            done += 1;
            let here = index[&(a, b)];
            let filled = match (&self.states[a], &other.states[b]) {
                (
                    State::Byte { class: ca, next: na },
                    State::Byte { class: cb, next: nb },
                ) => {
                    let class = (*ca).intersect(*cb);
                    let (na, nb) = (*na, *nb);
                    if class.is_empty() {
                        dead.clone()
                    } else {
                        mine.clear();
                        mine_on.iter_mut().for_each(|m| *m = false);
                        self.follow(na, &mut mine, &mut mine_on);
                        theirs.clear();
                        theirs_on.iter_mut().for_each(|m| *m = false);
                        other.follow(nb, &mut theirs, &mut theirs_on);
                        let mut kids: Vec<usize> = Vec::new();
                        for &x in &mine {
                            for &y in &theirs {
                                kids.push(pair(x, y, &mut states, &mut index, &mut queue));
                            }
                        }
                        let to = fan(&kids, 0, &mut states);
                        State::Byte { class, next: to }
                    }
                }
                (left, right) => match (condition_of(left), condition_of(right)) {
                    (Some((fa, ea)), Some((fb, eb))) => {
                        let follow = fa.intersect(fb);
                        let at_end = ea && eb;
                        if follow == ByteClass::any() && at_end {
                            State::Accept(id)
                        } else {
                            State::AcceptIf { id, follow, at_end }
                        }
                    }
                    _ => dead.clone(),
                },
            };
            states[here] = filled;
        }
        ByteNfa { states, start }
    }

    /// Which recognizer `s` accepts for against the byte a match would be
    /// followed by, where it accepts at all.
    fn accepts(&self, s: usize, after: Option<u8>) -> Option<u32> {
        match &self.states[s] {
            State::Accept(id) => Some(*id),
            State::AcceptIf { id, follow, at_end } => match after {
                Some(b) if follow.has(b) => Some(*id),
                None if *at_end => Some(*id),
                _ => None,
            },
            State::Byte { .. } | State::Split(..) => None,
        }
    }
}

/// One state reaching every state in `kids`, as a chain of splits, or `dead`
/// where there are none to reach.
fn fan(kids: &[usize], dead: usize, states: &mut Vec<State>) -> usize {
    match kids {
        [] => dead,
        [one] => *one,
        _ => {
            let mut at = kids[kids.len() - 1];
            for &k in kids[..kids.len() - 1].iter().rev() {
                states.push(State::Split(k, at));
                at = states.len() - 1;
            }
            at
        }
    }
}

/// The condition an accepting state carries - the class the byte after the
/// match must be in, and whether an end of input satisfies it - or `None` where
/// the state does not accept.
fn condition_of(s: &State) -> Option<(ByteClass, bool)> {
    match s {
        State::Accept(_) => Some((ByteClass::any(), true)),
        State::AcceptIf { follow, at_end, .. } => Some((*follow, *at_end)),
        State::Byte { .. } | State::Split(..) => None,
    }
}

/// Builds a [`ByteNfa`] one piece at a time.
///
/// The pieces are the Thompson forms: a class, a concatenation, an alternation,
/// and the three quantifiers. Each returns the piece's entry state and leaves
/// its exits dangling for whatever follows to patch - so a construction is one
/// pass and no piece has to know what comes after it.
#[derive(Default)]
pub struct Builder {
    states: Vec<State>,
}

/// A piece under construction: where it starts, and the exits still to patch.
pub struct Piece {
    start: usize,
    holes: Vec<Hole>,
}

/// Which side of which state still needs its target.
///
/// A split's left branch is never a hole: `or` knows both of its branches when
/// it makes the split, and every other split is a quantifier whose left branch
/// is the piece it repeats. Only the right branch is ever left dangling.
enum Hole {
    Next(usize),
    Right(usize),
}

impl Builder {
    /// A builder holding no state.
    #[must_use]
    pub fn new() -> Builder {
        Builder { states: Vec::new() }
    }

    /// A piece that consumes one byte of `class`.
    pub fn class(&mut self, class: ByteClass) -> Piece {
        let at = self.states.len();
        self.states.push(State::Byte { class, next: usize::MAX });
        Piece { start: at, holes: vec![Hole::Next(at)] }
    }

    /// `first` then `then`.
    pub fn then(&mut self, first: Piece, then: Piece) -> Piece {
        self.patch(&first.holes, then.start);
        Piece { start: first.start, holes: then.holes }
    }

    /// `left` or `right`, with `left` preferred.
    pub fn or(&mut self, left: Piece, right: Piece) -> Piece {
        let at = self.states.len();
        self.states.push(State::Split(left.start, right.start));
        let mut holes = left.holes;
        holes.extend(right.holes);
        Piece { start: at, holes }
    }

    /// `inner`, one time or more.
    pub fn plus(&mut self, inner: Piece) -> Piece {
        let at = self.states.len();
        self.states.push(State::Split(inner.start, usize::MAX));
        self.patch(&inner.holes, at);
        Piece { start: inner.start, holes: vec![Hole::Right(at)] }
    }

    /// `inner`, any number of times including none.
    pub fn star(&mut self, inner: Piece) -> Piece {
        let at = self.states.len();
        self.states.push(State::Split(inner.start, usize::MAX));
        self.patch(&inner.holes, at);
        Piece { start: at, holes: vec![Hole::Right(at)] }
    }

    /// `inner`, or nothing.
    pub fn maybe(&mut self, inner: Piece) -> Piece {
        let at = self.states.len();
        self.states.push(State::Split(inner.start, usize::MAX));
        let mut holes = inner.holes;
        holes.push(Hole::Right(at));
        Piece { start: at, holes }
    }

    /// Close `piece` with an acceptance carrying `id`, and hand back the
    /// automaton.
    #[must_use]
    pub fn accept(mut self, piece: Piece, id: u32) -> ByteNfa {
        let at = self.states.len();
        self.states.push(State::Accept(id));
        self.patch(&piece.holes, at);
        ByteNfa { states: self.states, start: piece.start }
    }

    /// Close `piece` with an acceptance carrying `id`, leaving a piece with no
    /// exits of its own.
    ///
    /// What lets several recognizers share one automaton: each is closed with
    /// its own acceptance and the closed pieces are alternated, so a run enters
    /// all of them at once and [`ByteNfa::recognize`] settles which one owns it.
    pub fn accepting(&mut self, piece: Piece, id: u32) -> Piece {
        let at = self.states.len();
        self.states.push(State::Accept(id));
        self.patch(&piece.holes, at);
        Piece { start: piece.start, holes: Vec::new() }
    }

    /// Close `piece` with an acceptance carrying `id` that holds only where the
    /// byte after the match is one `follow` holds, or the match reaches the end
    /// of the input and `at_end` admits an end there, and hand back the
    /// automaton.
    #[must_use]
    pub fn accept_if(mut self, piece: Piece, id: u32, follow: ByteClass, at_end: bool) -> ByteNfa {
        let at = self.states.len();
        self.states.push(State::AcceptIf { id, follow, at_end });
        self.patch(&piece.holes, at);
        ByteNfa { states: self.states, start: piece.start }
    }

    /// The conditional acceptance of [`Builder::accept_if`], leaving a piece
    /// with no exits of its own so several can be alternated into one
    /// automaton.
    pub fn accepting_if(
        &mut self,
        piece: Piece,
        id: u32,
        follow: ByteClass,
        at_end: bool,
    ) -> Piece {
        let at = self.states.len();
        self.states.push(State::AcceptIf { id, follow, at_end });
        self.patch(&piece.holes, at);
        Piece { start: piece.start, holes: Vec::new() }
    }

    /// Copy `nfa`'s states into this build and hand back the piece they form.
    ///
    /// The piece carries its own acceptances and has no exits, like one closed
    /// by [`Builder::accepting`], so an automaton built elsewhere - by
    /// [`ByteNfa::intersect`], which cannot be spelled as a piece - can be
    /// alternated with pieces built here.
    pub fn adopt(&mut self, nfa: &ByteNfa) -> Piece {
        let base = self.states.len();
        for s in nfa.states() {
            let shifted = match s {
                State::Byte { class, next } => State::Byte { class: *class, next: next + base },
                State::Split(a, b) => State::Split(a + base, b + base),
                State::Accept(id) => State::Accept(*id),
                State::AcceptIf { id, follow, at_end } => {
                    State::AcceptIf { id: *id, follow: *follow, at_end: *at_end }
                }
            };
            self.states.push(shifted);
        }
        Piece { start: nfa.start() + base, holes: Vec::new() }
    }

    /// Hand back the automaton beginning at `piece`, which must already carry
    /// its own acceptances.
    #[must_use]
    pub fn build(self, piece: Piece) -> ByteNfa {
        ByteNfa { states: self.states, start: piece.start }
    }

    /// Point every hole at `to`.
    fn patch(&mut self, holes: &[Hole], to: usize) {
        for h in holes {
            match *h {
                Hole::Next(s) => {
                    if let State::Byte { next, .. } = &mut self.states[s] {
                        *next = to;
                    }
                }
                Hole::Right(s) => {
                    if let State::Split(_, b) = &mut self.states[s] {
                        *b = to;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Digits, the simplest recognizer shape the lexer has.
    fn digits() -> ByteNfa {
        let mut b = Builder::new();
        let d = b.class(ByteClass::range(b'0', b'9'));
        let run = b.plus(d);
        b.accept(run, 0)
    }

    /// A number as the lexer reads one: digits, then optionally a point and
    /// more digits. The two readings share a prefix, so which one is kept is
    /// decided by extent rather than by an order of trying.
    fn number() -> ByteNfa {
        let mut b = Builder::new();
        let d1 = b.class(ByteClass::range(b'0', b'9'));
        let whole = b.plus(d1);
        let point = b.class(ByteClass::just(b'.'));
        let d2 = b.class(ByteClass::range(b'0', b'9'));
        let frac_digits = b.plus(d2);
        let frac = b.then(point, frac_digits);
        let maybe_frac = b.maybe(frac);
        let all = b.then(whole, maybe_frac);
        b.accept(all, 0)
    }

    #[test]
    fn a_class_holds_what_it_was_given_and_nothing_else() {
        let c = ByteClass::range(b'0', b'9');
        assert!(c.has(b'0') && c.has(b'9') && c.has(b'5'));
        assert!(!c.has(b'/') && !c.has(b':') && !c.has(b'a'));
        assert!(ByteClass::none().is_empty());
        assert!(!ByteClass::any().is_empty());
        assert!(ByteClass::just(b'x').negate().has(b'y'));
        assert!(!ByteClass::just(b'x').negate().has(b'x'));
        assert!(ByteClass::just(b'a').union(ByteClass::just(b'b')).has(b'b'));
    }

    #[test]
    fn a_run_takes_every_byte_it_can_and_stops_where_it_cannot() {
        let n = digits();
        assert_eq!(n.recognize(b"123 rest"), Some((3, 0)));
        assert_eq!(n.recognize(b"7"), Some((1, 0)));
        assert_eq!(n.recognize(b"abc"), None);
        assert_eq!(n.recognize(b""), None);
    }

    #[test]
    fn the_longer_reading_wins_where_two_share_a_prefix() {
        let n = number();
        // The whole-number reading accepts at 1 and the fractional one at 3,
        // and the longer is the token.
        assert_eq!(n.recognize(b"1.5"), Some((3, 0)));
        // A point with no digit after it is not part of the number, so the
        // acceptance at 1 is the one kept.
        assert_eq!(n.recognize(b"1. "), Some((1, 0)));
        assert_eq!(n.recognize(b"12.34.56"), Some((5, 0)));
        assert_eq!(n.recognize(b".5"), None);
    }

    #[test]
    fn a_star_accepts_the_empty_prefix_and_a_plus_does_not() {
        let mut b = Builder::new();
        let d = b.class(ByteClass::range(b'0', b'9'));
        let run = b.star(d);
        let starred = b.accept(run, 0);
        assert_eq!(starred.recognize(b"abc"), Some((0, 0)));
        assert_eq!(starred.recognize(b"12abc"), Some((2, 0)));
        assert_eq!(digits().recognize(b"abc"), None);
    }

    #[test]
    fn an_alternation_reports_which_branch_accepted() {
        let mut b = Builder::new();
        let word = b.class(ByteClass::range(b'a', b'z'));
        let word_run = b.plus(word);
        let word_end = b.accept(word_run, 1);
        assert_eq!(word_end.recognize(b"abc1"), Some((3, 1)));
    }

    /// A run of letters that is a match only where nothing alphanumeric follows
    /// it, which is the shape a trailing context has.
    fn word_at_a_boundary() -> ByteNfa {
        let mut b = Builder::new();
        let c = b.class(ByteClass::range(b'a', b'z'));
        let run = b.plus(c);
        let alnum = ByteClass::range(b'a', b'z')
            .union(ByteClass::range(b'A', b'Z'))
            .union(ByteClass::range(b'0', b'9'));
        b.accept_if(run, 0, alnum.negate(), true)
    }

    /// What follows a run decides whether it is a match, and the end of the
    /// input is an answer of its own rather than a byte that fails every class.
    ///
    /// The last two cases are why a condition cannot be a rule applied to a
    /// match the automaton already chose: the byte that refuses the longest
    /// reading refuses every shorter one too, so there is no acceptance left to
    /// fall back to and the run is not this recognizer's at all.
    #[test]
    fn what_follows_a_run_decides_whether_it_accepts() {
        let n = word_at_a_boundary();
        assert_eq!(n.recognize(b"abc"), Some((3, 0)), "the input ends after it");
        assert_eq!(n.recognize(b"abc "), Some((3, 0)), "a space is not alphanumeric");
        assert_eq!(n.recognize(b"abc1"), None, "a digit runs it on");
        assert_eq!(n.recognize(b"abcD"), None, "and so does a letter it cannot take");
    }

    /// A lower recognizer id wins although the other reaches further, which is
    /// the rule a lexer chooses a kind by and not a regular expression's.
    ///
    /// The shorter branch here is the one that would lose under a longest match,
    /// so the two rules disagree about this automaton and the assertion says
    /// which one is implemented. `lexer::try_typed_token_chain` is the reason
    /// it is this one: it takes the first branch of its chain that matches at
    /// all, so a hash digest beats the longer base64 blob that starts with it.
    #[test]
    fn precedence_decides_before_reach_does() {
        let mut b = Builder::new();
        let short = b.class(ByteClass::range(b'a', b'z'));
        let short_run = b.plus(short);
        let short_end = b.accepting(short_run, 0);
        let long = b.class(ByteClass::range(b'a', b'z').union(ByteClass::just(b'-')));
        let long_run = b.plus(long);
        let long_end = b.accepting(long_run, 1);
        let both = b.or(short_end, long_end);
        let n = b.build(both);
        assert_eq!(n.recognize(b"abc-def"), Some((3, 0)), "the longer branch took the run");
        assert_eq!(n.recognize(b"-def"), Some((4, 1)), "only the lower branch can start here");
    }

    /// The hex digits, either case.
    fn hex() -> ByteClass {
        ByteClass::range(b'0', b'9')
            .union(ByteClass::range(b'a', b'f'))
            .union(ByteClass::range(b'A', b'F'))
    }

    /// Exactly four hex bytes, which is a shape.
    fn four_hex() -> ByteNfa {
        let mut b = Builder::new();
        let mut run = b.class(hex());
        for _ in 1..4 {
            let next = b.class(hex());
            run = b.then(run, next);
        }
        b.accept(run, 0)
    }

    /// A hex run holding a hex letter somewhere, which is a condition.
    fn holds_a_letter() -> ByteNfa {
        let mut b = Builder::new();
        let before = b.class(hex());
        let head = b.star(before);
        let letter = b.class(ByteClass::range(b'a', b'f').union(ByteClass::range(b'A', b'F')));
        let after = b.class(hex());
        let tail = b.star(after);
        let led = b.then(head, letter);
        let all = b.then(led, tail);
        b.accept(all, 0)
    }

    /// A meet accepts where both sides accept and nowhere else, which is what
    /// lets a shape and a condition over one run be built apart.
    ///
    /// Neither side alone is the recognizer. The shape takes four hex bytes
    /// whether or not a letter is among them, and the condition takes any hex
    /// run that holds one, of any length - so `0000` is a shape and not a
    /// condition, `00a` is a condition and not a shape, and only `00a0` is both.
    #[test]
    fn a_meet_accepts_what_both_of_its_sides_accept() {
        let both = four_hex().intersect(&holds_a_letter(), 7);
        assert_eq!(both.recognize(b"00a0"), Some((4, 7)), "four hex with a letter among them");
        assert_eq!(both.recognize(b"abcd"), Some((4, 7)), "four hex, every one a letter");
        assert_eq!(both.recognize(b"0000"), None, "the shape holds and the condition does not");
        assert_eq!(both.recognize(b"00a"), None, "the condition holds and the shape does not");
        assert_eq!(both.recognize(b"00a00"), Some((4, 7)), "the shape ends it at four");
        assert_eq!(both.recognize(b"zzzz"), None, "neither side can start");
    }

    /// An automaton whose epsilon edges form a cycle must still terminate,
    /// which is what the mark array in `follow` is for: a star over something
    /// that can match nothing loops back to itself without reading a byte.
    #[test]
    fn a_loop_over_an_empty_piece_terminates() {
        let mut b = Builder::new();
        let d = b.class(ByteClass::range(b'0', b'9'));
        let inner = b.maybe(d);
        let outer = b.star(inner);
        let n = b.accept(outer, 0);
        assert_eq!(n.recognize(b"12"), Some((2, 0)));
        assert_eq!(n.recognize(b"ab"), Some((0, 0)));
    }
}
