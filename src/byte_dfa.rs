//! The determinizer over a byte-grain automaton, built as it is walked.
//!
//! A simulation carries a set of states a byte and re-derives that set at every
//! position, so a state it has already been in costs the same the second time.
//! A determinized automaton derives a set once: the set becomes a state, and
//! the byte that leaves it is a table lookup. Over an input where the same few
//! sets recur - which is what lexing text is - the second visit is the common
//! one.
//!
//! Built as it is walked rather than up front, because the subset construction
//! is exponential in the worst case and the sets an input actually reaches are
//! not. A pattern whose full determinization would not fit still runs, at the
//! simulation's cost for the sets it reaches once and the table's for the rest.

use std::collections::HashMap;

use crate::byte_nfa::{ByteClass, ByteNfa, State};

/// A transition not yet computed. Distinct from [`DEAD`] because "nothing
/// leaves this state on this byte" is an answer and "nobody has asked" is not,
/// and a cache that conflated them would recompute every dead end forever.
const UNKNOWN: u32 = u32::MAX;

/// No state at all leaves here on this byte; the run ends.
const DEAD: u32 = u32::MAX - 1;

/// One determinized state: where each byte leads, and what it accepts.
struct DfaState {
    /// Where each byte leads, filled as bytes are asked about.
    next: Box<[u32; 256]>,
    /// Which recognizer accepts here whatever follows, if any.
    accept: Option<u32>,
    /// The acceptances here that hold only against the byte a match would be
    /// followed by, which is a byte later than the one that reached this state
    /// and so cannot be settled when the state is made.
    conditional: Vec<(u32, ByteClass, bool)>,
    /// The automaton states this stands for, kept so a byte that has not been
    /// asked about can be answered without re-deriving the set.
    set: Vec<usize>,
}

/// A byte automaton determinized on demand.
pub struct ByteDfa<'n> {
    nfa: &'n ByteNfa,
    states: Vec<DfaState>,
    /// Which determinized state a set of automaton states is, so two paths
    /// reaching the same set share one.
    index: HashMap<Vec<usize>, u32>,
}

impl<'n> ByteDfa<'n> {
    /// A determinizer over `nfa`, holding only its start state.
    #[must_use]
    pub fn new(nfa: &'n ByteNfa) -> ByteDfa<'n> {
        let mut dfa = ByteDfa { nfa, states: Vec::new(), index: HashMap::new() };
        let start = dfa.closure_of(&[nfa.start()]);
        dfa.intern(start);
        dfa
    }

    /// How many determinized states have been built so far, which is how much
    /// of the construction the inputs seen have actually reached.
    #[must_use]
    pub fn built(&self) -> usize {
        self.states.len()
    }

    /// The token `input` begins with, as the recognizer that owns it and the
    /// byte it ends at, or `None` where no recognizer claims one.
    ///
    /// The same answer [`ByteNfa::recognize`] gives, by the same rule - the
    /// lowest recognizer id that accepts anywhere wins, and a longer reach wins
    /// only among acceptances carrying that id.
    pub fn recognize(&mut self, input: &[u8]) -> Option<(usize, u32)> {
        let mut at = 0u32;
        let mut best = self.accepts(0, input.first().copied()).map(|id| (0, id));
        for (i, &b) in input.iter().enumerate() {
            let to = self.step(at, b);
            if to == DEAD {
                break;
            }
            at = to;
            if let Some(id) = self.accepts(at, input.get(i + 1).copied()) {
                best = match best {
                    Some((_, who)) if who < id => best,
                    Some((end, who)) if who == id && end > i => best,
                    _ => Some((i + 1, id)),
                };
            }
        }
        best
    }

    /// Where `from` leads on `b`, computing and caching it where this is the
    /// first ask.
    fn step(&mut self, from: u32, b: u8) -> u32 {
        let held = self.states[from as usize].next[b as usize];
        if held != UNKNOWN {
            return held;
        }
        let mut moved: Vec<usize> = Vec::new();
        for &s in &self.states[from as usize].set {
            if let State::Byte { class, next } = &self.nfa.states()[s]
                && class.has(b)
            {
                moved.push(*next);
            }
        }
        let to = if moved.is_empty() {
            DEAD
        } else {
            let set = self.closure_of(&moved);
            self.intern(set)
        };
        self.states[from as usize].next[b as usize] = to;
        to
    }

    /// The determinized state for `set`, made where it is new.
    fn intern(&mut self, set: Vec<usize>) -> u32 {
        if let Some(&at) = self.index.get(&set) {
            return at;
        }
        let mut accept: Option<u32> = None;
        let mut conditional: Vec<(u32, ByteClass, bool)> = Vec::new();
        for &s in &set {
            match &self.nfa.states()[s] {
                State::Accept(id) => accept = Some(accept.map_or(*id, |held: u32| held.min(*id))),
                State::AcceptIf { id, follow, at_end } => conditional.push((*id, *follow, *at_end)),
                State::Byte { .. } | State::Split(..) => {}
            }
        }
        let at = u32::try_from(self.states.len()).expect("a state count within the table's width");
        self.index.insert(set.clone(), at);
        self.states.push(DfaState { next: Box::new([UNKNOWN; 256]), accept, conditional, set });
        at
    }

    /// Which recognizer accepts in `s` against the byte a match would be
    /// followed by, where one does.
    ///
    /// The lower id wins among several, which is declaration order, and a
    /// conditional acceptance counts only where its condition holds.
    fn accepts(&self, s: u32, after: Option<u8>) -> Option<u32> {
        let held = &self.states[s as usize];
        let mut best = held.accept;
        for &(id, follow, at_end) in &held.conditional {
            let holds = match after {
                Some(b) => follow.has(b),
                None => at_end,
            };
            if holds {
                best = Some(best.map_or(id, |who: u32| who.min(id)));
            }
        }
        best
    }

    /// Every state reachable from `seeds` without reading a byte, in order.
    ///
    /// Sorted and deduplicated, because the set is the cache's key: two paths
    /// that reach the same states by different routes have to hash alike or the
    /// construction grows a state per route rather than per set.
    fn closure_of(&self, seeds: &[usize]) -> Vec<usize> {
        let mut on = vec![false; self.nfa.len()];
        let mut out: Vec<usize> = Vec::new();
        let mut stack: Vec<usize> = seeds.to_vec();
        while let Some(t) = stack.pop() {
            if on[t] {
                continue;
            }
            on[t] = true;
            match self.nfa.states()[t] {
                State::Split(a, b) => {
                    stack.push(b);
                    stack.push(a);
                }
                State::Byte { .. } | State::Accept(_) | State::AcceptIf { .. } => out.push(t),
            }
        }
        out.sort_unstable();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::byte_nfa::{Builder, ByteClass};

    /// Digits, then optionally a point and more digits.
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

    /// Lower-case letters and underscore, one or more.
    fn word() -> ByteNfa {
        let mut b = Builder::new();
        let c = b.class(ByteClass::range(b'a', b'z').union(ByteClass::just(b'_')));
        let run = b.plus(c);
        b.accept(run, 1)
    }

    /// Letters and underscore, accepted only where no digit follows.
    ///
    /// The case the table cannot settle when it makes a state: whether this
    /// accepts depends on a byte one later than the one that reached the state,
    /// so it is read at the match and not at the intern.
    fn word_not_before_digit() -> ByteNfa {
        let mut b = Builder::new();
        let c = b.class(ByteClass::range(b'a', b'z').union(ByteClass::just(b'_')));
        let run = b.plus(c);
        b.accept_if(run, 1, ByteClass::range(b'0', b'9').negate(), true)
    }

    /// Exactly four hex bytes, at least one of them a letter, built as a meet of
    /// a shape and a condition.
    ///
    /// The case the determinizer has most to get wrong: a product automaton is
    /// dense with splits, so a closure reaches many states at once and two
    /// inputs arrive at one set by different routes.
    fn four_hex_with_a_letter() -> ByteNfa {
        let hex = ByteClass::range(b'0', b'9')
            .union(ByteClass::range(b'a', b'f'))
            .union(ByteClass::range(b'A', b'F'));
        let shape = {
            let mut b = Builder::new();
            let mut run = b.class(hex);
            for _ in 1..4 {
                let next = b.class(hex);
                run = b.then(run, next);
            }
            b.accept(run, 0)
        };
        let condition = {
            let mut b = Builder::new();
            let before = b.class(hex);
            let head = b.star(before);
            let letter = b.class(ByteClass::range(b'a', b'f').union(ByteClass::range(b'A', b'F')));
            let after = b.class(hex);
            let tail = b.star(after);
            let led = b.then(head, letter);
            let all = b.then(led, tail);
            b.accept(all, 0)
        };
        shape.intersect(&condition, 0)
    }

    /// The property the determinizer exists to keep: it answers what the
    /// simulation answers, on every input, always. A determinizer that is
    /// faster and disagrees is not a determinizer.
    #[test]
    fn the_table_answers_what_the_simulation_answers() {
        let inputs: [&[u8]; 14] = [
            b"",
            b"1",
            b"1.5",
            b"1. ",
            b"12.34.56",
            b".5",
            b"abc",
            b"a_b1",
            b"_",
            b"999999999",
            b"0.0.0.0",
            b"1a",
            b"zzz ",
            b"3.14159265358979",
        ];
        for nfa in [number(), word(), word_not_before_digit(), four_hex_with_a_letter()] {
            let mut dfa = ByteDfa::new(&nfa);
            for inp in inputs {
                assert_eq!(
                    dfa.recognize(inp),
                    nfa.recognize(inp),
                    "{:?} on a {}-state automaton",
                    String::from_utf8_lossy(inp),
                    nfa.len()
                );
            }
        }
    }

    /// A set reached twice is one state, not two, which is what makes the
    /// construction finite on an input that repeats.
    #[test]
    fn a_set_reached_twice_is_interned_once() {
        let nfa = number();
        let mut dfa = ByteDfa::new(&nfa);
        dfa.recognize(b"1234567890");
        let after_first = dfa.built();
        for _ in 0..64 {
            dfa.recognize(b"1234567890");
        }
        assert_eq!(dfa.built(), after_first, "walking the same bytes again built new states");
    }

    /// The construction grows only where an input reaches somewhere new, which
    /// is what "as it is walked" means and what keeps a wide automaton usable.
    #[test]
    fn the_table_grows_only_where_an_input_goes() {
        let nfa = number();
        let mut dfa = ByteDfa::new(&nfa);
        let at_rest = dfa.built();
        assert_eq!(at_rest, 1, "a fresh determinizer holds its start and nothing else");
        dfa.recognize(b"7");
        let after_digits = dfa.built();
        assert!(after_digits > at_rest, "a digit reaches a state the start is not");
        dfa.recognize(b"7.7");
        assert!(dfa.built() > after_digits, "a fraction reaches further than a whole number");
    }

    /// A dead end is remembered as one. Recomputing it would make a run over
    /// bytes the automaton refuses cost what deriving the set costs, every
    /// time, which is the whole of what the cache is for.
    #[test]
    fn a_dead_end_is_cached_rather_than_rederived() {
        let nfa = word();
        let mut dfa = ByteDfa::new(&nfa);
        dfa.recognize(b"abc");
        let built = dfa.built();
        for _ in 0..64 {
            assert_eq!(dfa.recognize(b"111"), None);
        }
        assert_eq!(dfa.built(), built, "a refused byte built a state");
    }
}
