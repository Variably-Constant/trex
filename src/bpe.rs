//! Byte-pair encoding: a learned subword tokenizer (Sennrich, Haddow,
//! Birch, ACL 2016), the data-driven token inventory for natural-language
//! text where the rule-based lexer's whole-token boundaries do not fit.
//!
//! Training learns an ordered list of merge operations from a corpus: each
//! word is its characters plus an end-of-word marker `</w>`, weighted by
//! frequency, and the most frequent adjacent symbol pair is merged into one
//! symbol, repeated `num_merges` times. Encoding a word splits it into
//! characters and applies the learned merges in order, so any word -- even
//! one unseen in training -- segments into known subword units. Pairs never
//! cross word boundaries, so the work runs over the distinct words weighted
//! by count.

use std::collections::{HashMap, HashSet};

/// The end-of-word marker appended to every word before merging, so a
/// subword that ends a word is distinct from the same letters mid-word.
const END: &str = "</w>";

/// The first `cap` bytes of `corpus` cut back to the whitespace at or before
/// the cut, so a training sample of a large corpus trains in bounded time
/// and ends on a whole word; all of `corpus` where it is no longer than
/// `cap`, and its first byte where no whitespace comes before the cut.
#[must_use]
pub fn sample(corpus: &[u8], cap: usize) -> &[u8] {
    if corpus.len() <= cap {
        return corpus;
    }
    let mut end = cap;
    while end > 0 && !corpus[end].is_ascii_whitespace() {
        end -= 1;
    }
    &corpus[..end.max(1)]
}

/// A trained byte-pair encoder: the ordered merge operations and their
/// ranks (learned order), which encoding applies greedily.
#[derive(Clone, Debug, Default)]
pub struct Bpe {
    merges: Vec<(String, String)>,
    ranks: HashMap<(String, String), usize>,
}

impl Bpe {
    /// Learn `num_merges` merge operations from `corpus`. Words are split on
    /// whitespace and weighted by frequency; the most frequent adjacent pair
    /// is merged each round, ties broken by the pair lexicographically so
    /// the model is reproducible.
    #[must_use]
    pub fn train(corpus: &[u8], num_merges: usize) -> Bpe {
        let text = String::from_utf8_lossy(corpus);
        let mut freq: HashMap<&str, u64> = HashMap::new();
        for word in text.split_whitespace() {
            *freq.entry(word).or_default() += 1;
        }
        // Each distinct word as its character symbols plus the end marker.
        let mut words: Vec<(Vec<String>, u64)> = freq
            .into_iter()
            .map(|(w, f)| {
                let mut syms: Vec<String> = w.chars().map(|c| c.to_string()).collect();
                syms.push(END.to_string());
                (syms, f)
            })
            .collect();

        // Index each pair's total count and the words it occurs in, updated
        // incrementally so a merge re-counts only the words it touches rather
        // than the whole corpus (the paper's efficiency note).
        let mut pair_count: HashMap<(String, String), u64> = HashMap::new();
        let mut pair_words: HashMap<(String, String), HashSet<usize>> = HashMap::new();
        for idx in 0..words.len() {
            add_pairs(&words, idx, &mut pair_count, &mut pair_words);
        }

        let mut merges: Vec<(String, String)> = Vec::new();
        for _ in 0..num_merges {
            // Greatest count; tie broken by the lexicographically smaller
            // pair (it compares as greater, so `max_by` selects it).
            let Some(best) = pair_count
                .iter()
                .max_by(|(pa, ca), (pb, cb)| ca.cmp(cb).then_with(|| pb.cmp(pa)))
                .map(|(p, _)| p.clone())
            else {
                break;
            };
            let joined = format!("{}{}", best.0, best.1);
            let affected: Vec<usize> =
                pair_words.get(&best).map(|s| s.iter().copied().collect()).unwrap_or_default();
            for idx in affected {
                remove_pairs(&words, idx, &mut pair_count, &mut pair_words);
                merge_in_word(&mut words[idx].0, &best, &joined);
                add_pairs(&words, idx, &mut pair_count, &mut pair_words);
            }
            merges.push(best);
        }

        let ranks = merges.iter().cloned().enumerate().map(|(r, p)| (p, r)).collect();
        Bpe { merges, ranks }
    }

    /// Tokenize `text` into subword units: each whitespace word is split
    /// into characters and the learned merges are applied greedily (lowest
    /// rank first), so every token is a known subword ending in `</w>` at a
    /// word boundary.
    #[must_use]
    pub fn encode(&self, text: &[u8]) -> Vec<String> {
        let text = String::from_utf8_lossy(text);
        let mut out = Vec::new();
        for word in text.split_whitespace() {
            out.extend(self.encode_word(word));
        }
        out
    }

    /// The subword segmentation of one word.
    fn encode_word(&self, word: &str) -> Vec<String> {
        let mut syms: Vec<String> = word.chars().map(|c| c.to_string()).collect();
        if syms.is_empty() {
            return syms;
        }
        syms.push(END.to_string());
        loop {
            // The adjacent pair with the lowest learned rank, if any.
            let mut best: Option<(usize, usize)> = None; // (rank, index)
            for i in 0..syms.len() - 1 {
                if let Some(&rank) = self.ranks.get(&(syms[i].clone(), syms[i + 1].clone()))
                    && best.is_none_or(|(br, _)| rank < br)
                {
                    best = Some((rank, i));
                }
            }
            let Some((_, i)) = best else {
                break;
            };
            syms[i] = format!("{}{}", syms[i], syms[i + 1]);
            syms.remove(i + 1);
        }
        syms
    }

    /// The number of learned merge operations (the subword vocabulary grows
    /// by one per merge over the base characters).
    #[must_use]
    pub fn len(&self) -> usize {
        self.merges.len()
    }

    /// Whether no merges were learned.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.merges.is_empty()
    }

    /// Serialize the model: one merge per line, the two symbols tab-
    /// separated, in learned order.
    #[must_use]
    pub fn to_lines(&self) -> String {
        let mut s = String::new();
        for (a, b) in &self.merges {
            s.push_str(a);
            s.push('\t');
            s.push_str(b);
            s.push('\n');
        }
        s
    }

    /// Parse a model written by [`Bpe::to_lines`], refusing a line that is
    /// not two symbols separated by a tab; a blank line is passed over.
    ///
    /// # Errors
    ///
    /// The first line holding something other than whitespace and no tab,
    /// by its number counting from 1.
    pub fn parse(s: &str) -> Result<Bpe, String> {
        let mut merges: Vec<(String, String)> = Vec::new();
        for (i, line) in s.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let Some((a, b)) = line.split_once('\t') else {
                return Err(format!("line {}: a model line is two symbols separated by a tab", i + 1));
            };
            merges.push((a.to_string(), b.to_string()));
        }
        let ranks = merges.iter().cloned().enumerate().map(|(r, p)| (p, r)).collect();
        Ok(Bpe { merges, ranks })
    }

    /// Parse a model written by [`Bpe::to_lines`]. Lines without a tab are
    /// skipped; [`Bpe::parse`] refuses them instead.
    #[must_use]
    pub fn from_lines(s: &str) -> Bpe {
        let merges: Vec<(String, String)> = s
            .lines()
            .filter_map(|l| l.split_once('\t').map(|(a, b)| (a.to_string(), b.to_string())))
            .collect();
        let ranks = merges.iter().cloned().enumerate().map(|(r, p)| (p, r)).collect();
        Bpe { merges, ranks }
    }
}

/// Add word `idx`'s adjacent pairs into the indices: each pair gains the
/// word's frequency and records the word.
fn add_pairs(
    words: &[(Vec<String>, u64)],
    idx: usize,
    pair_count: &mut HashMap<(String, String), u64>,
    pair_words: &mut HashMap<(String, String), HashSet<usize>>,
) {
    let (syms, f) = &words[idx];
    for w in syms.windows(2) {
        let p = (w[0].clone(), w[1].clone());
        *pair_count.entry(p.clone()).or_default() += f;
        pair_words.entry(p).or_default().insert(idx);
    }
}

/// Remove word `idx`'s adjacent pairs from the indices (the inverse of
/// [`add_pairs`]); a pair whose count reaches zero is dropped so it is never
/// selected as the most frequent.
fn remove_pairs(
    words: &[(Vec<String>, u64)],
    idx: usize,
    pair_count: &mut HashMap<(String, String), u64>,
    pair_words: &mut HashMap<(String, String), HashSet<usize>>,
) {
    let (syms, f) = &words[idx];
    for w in syms.windows(2) {
        let p = (w[0].clone(), w[1].clone());
        if let Some(c) = pair_count.get_mut(&p) {
            *c = c.saturating_sub(*f);
            if *c == 0 {
                pair_count.remove(&p);
            }
        }
        if let Some(s) = pair_words.get_mut(&p) {
            s.remove(&idx);
        }
    }
}

/// Replace every adjacent `pair` in `syms` with the single symbol `joined`.
fn merge_in_word(syms: &mut Vec<String>, pair: &(String, String), joined: &str) {
    let mut i = 0;
    while i + 1 < syms.len() {
        if syms[i] == pair.0 && syms[i + 1] == pair.1 {
            syms[i] = joined.to_string();
            syms.remove(i + 1);
        } else {
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_parses_back_and_a_line_with_no_tab_is_refused_by_number() {
        let model = Bpe::train(b"low low low lower lowest newer newest", 6);
        let read = Bpe::parse(&model.to_lines()).expect("a written model parses");
        assert_eq!(read.to_lines(), model.to_lines());
        assert_eq!(read.encode(b"lowest"), model.encode(b"lowest"));
        let e = Bpe::parse("l\to\n\nnot a merge\n").expect_err("a line with no tab");
        assert!(e.starts_with("line 3:"), "{e}");
    }

    #[test]
    fn a_sample_ends_on_a_whole_word() {
        assert_eq!(sample(b"alpha beta gamma", 8), b"alpha");
        assert_eq!(sample(b"alpha beta", 64), b"alpha beta");
        assert_eq!(sample(b"alphabet", 3), b"a");
    }

    #[test]
    fn learns_frequent_merges_and_segments() {
        // "low" appears far more than its letters do elsewhere, so the
        // characters of "low" merge first and it encodes as one subword.
        let corpus = b"low low low low low lower lower newest widest";
        let bpe = Bpe::train(corpus, 8);
        assert!(!bpe.is_empty());
        // "low" collapsed to a single subword unit ending the word.
        assert_eq!(bpe.encode_word("low"), vec!["low</w>".to_string()]);
        // An unseen word still segments into known subword pieces (open
        // vocabulary), and concatenating them with the marker stripped
        // reconstructs the word.
        let pieces = bpe.encode_word("lowest");
        let joined: String = pieces.join("").replace(END, "");
        assert_eq!(joined, "lowest");
    }

    #[test]
    fn merges_are_deterministic() {
        // Tie-broken by the pair, so two trainings give the identical model.
        let corpus = b"ab ab cd cd ab cd";
        let a = Bpe::train(corpus, 5);
        let b = Bpe::train(corpus, 5);
        assert_eq!(a.to_lines(), b.to_lines());
    }

    #[test]
    fn roundtrips_through_serialization() {
        let bpe = Bpe::train(b"the cat sat on the mat the cat ran", 10);
        let reloaded = Bpe::from_lines(&bpe.to_lines());
        assert_eq!(bpe.encode(b"the cat"), reloaded.encode(b"the cat"));
    }

    #[test]
    fn encoding_covers_every_character() {
        // Even with no useful merges, a word always segments (into its
        // characters), so encoding never drops input.
        let bpe = Bpe::train(b"a b c", 2);
        let pieces = bpe.encode_word("xyz");
        let joined: String = pieces.join("").replace(END, "");
        assert_eq!(joined, "xyz");
    }
}
