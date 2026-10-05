//! Edit distance over whole tokens: whether one text is within `k` edits of
//! another, an edit being one character inserted, deleted or substituted,
//! which is the Levenshtein distance ugrep's `-Z` and spaCy's `FUZZY` also
//! count. A transposition costs two.

/// Whether `a` and `b` are within `k` edits of each other, counted over
/// characters. The band the count can reach is `k` wide on either side of
/// the diagonal, so the work is the shorter length times `2k + 1`.
#[must_use]
pub fn within(a: &[u8], b: &[u8], k: u8) -> bool {
    let a: Vec<char> = String::from_utf8_lossy(a).chars().collect();
    let b: Vec<char> = String::from_utf8_lossy(b).chars().collect();
    let k = usize::from(k);
    if a.len().abs_diff(b.len()) > k {
        return false;
    }
    if k == 0 {
        return a == b;
    }
    // The distance from a prefix of `a` to a prefix of `b`, a row at a time,
    // over the band; a cell outside it is past `k` and never read.
    let past = k + 1;
    let mut prev: Vec<usize> = (0..=b.len()).map(|j| if j <= k { j } else { past }).collect();
    let mut cur = vec![past; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        let row = i + 1;
        let lo = row.saturating_sub(k);
        let hi = (row + k).min(b.len());
        cur[..=b.len()].fill(past);
        if lo == 0 {
            cur[0] = row;
        }
        for j in lo.max(1)..=hi {
            let sub = prev[j - 1] + usize::from(a[row - 1] != b[j - 1]);
            let del = prev[j] + 1;
            let ins = cur[j - 1] + 1;
            cur[j] = sub.min(del).min(ins).min(past);
        }
        if cur[lo..=hi].iter().all(|&d| d > k) {
            return false;
        }
        std::mem::swap(&mut prev, &mut cur);
        let _unused = ca;
    }
    prev[b.len()] <= k
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_edit_is_an_insertion_a_deletion_or_a_substitution() {
        assert!(within(b"color", b"colour", 1), "insertion");
        assert!(within(b"colour", b"color", 1), "deletion");
        assert!(within(b"color", b"colar", 1), "substitution");
        assert!(!within(b"color", b"colours", 1), "two edits");
        assert!(within(b"color", b"colours", 2));
        assert!(!within(b"the", b"teh", 1), "a transposition costs two");
        assert!(within(b"the", b"teh", 2));
        assert!(within(b"same", b"same", 0));
        assert!(!within(b"same", b"sane", 0));
        assert!(within(b"", b"a", 1));
        assert!(!within(b"abcdef", b"", 5));
    }

    #[test]
    fn edits_count_characters_not_bytes() {
        assert!(within("naïve".as_bytes(), b"naive", 1));
        assert!(within("résumé".as_bytes(), b"resume", 2));
        assert!(!within("résumé".as_bytes(), b"resume", 1));
    }
}
