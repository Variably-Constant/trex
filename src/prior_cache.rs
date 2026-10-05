//! The baked prior laid out on disk as the coder's tables, mapped instead of
//! decoded.
//!
//! `load_byte_ngram` decodes the blob and expands every row into a hash table,
//! about 1.9 s and a gigabyte resident for the shipped prior, in every
//! process that codes with it. This writes those rows once into open-addressing
//! tables - 16-byte slots of key and two counts, one file per order, capacity a
//! power of two at a 0.7 load factor - and maps them on later runs in under a
//! millisecond, with only the pages a read touches becoming resident. A read
//! costs 1.44x to 1.73x the hash table's, measured on the coder's own read
//! sequence, so mapping is faster on inputs under about 7-10 MB and the decode
//! is faster above; the caller chooses.
//!
//! A manifest beside the tables records the blob's length and hash, so a cache
//! built from another blob is not opened. A key of 0 cannot be stored - an
//! all-zero slot marks it empty - so a row with that key is kept in the
//! manifest and answered from there.
//!
//! Compiled with the `compress` feature, as the prior it maps is.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use memmap2::Mmap;

use crate::seam::{PreloadTable, PriorLookup, load_byte_ngram};

/// One stored row: the key and its two counts.
const SLOT: usize = 16;
const MANIFEST: &str = "prior.manifest";
const FORMAT: &str = "trex-prior-cache 1";

/// One order's table: its literal order, the mapped file, its slot count, and
/// the row whose key is 0 when the order has one.
struct MappedOrder {
    order: usize,
    map: Mmap,
    cap: usize,
    zero_row: Option<(u32, u32)>,
}

/// The baked prior read from mapped tables. See the module documentation.
pub struct MappedPrior {
    orders: Vec<MappedOrder>,
}

/// FNV-1a over the blob, the identity the manifest records.
fn blob_hash(blob: &[u8]) -> u64 {
    blob.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// Slots for `rows` at a load factor near 0.7, a power of two so the index is
/// a mask.
fn capacity_for(rows: usize) -> usize {
    ((rows * 10) / 7).next_power_of_two().max(16)
}

fn table_path(dir: &Path, order: usize) -> PathBuf {
    dir.join(format!("order{order}.tbl"))
}

fn bad(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

/// Word `i` of a manifest line as a number, naming the line on any failure.
fn field(words: &[&str], i: usize, what: &str, line: &str) -> io::Result<usize> {
    match words.get(i) {
        None => Err(bad(format!("manifest line {line:?} has no {what}"))),
        Some(s) => match s.parse::<usize>() {
            Ok(v) => Ok(v),
            Err(e) => Err(bad(format!("manifest line {line:?}: {what} {s:?} does not parse: {e}"))),
        },
    }
}

fn narrow(v: usize, what: &str, line: &str) -> io::Result<u32> {
    match u32::try_from(v) {
        Ok(n) => Ok(n),
        Err(e) => Err(bad(format!("manifest line {line:?}: {what} {v} does not fit a count: {e}"))),
    }
}

impl MappedPrior {
    /// Write `blob`'s rows into `dir` as mapped tables and a manifest, then
    /// open them. An existing cache in `dir` is overwritten.
    pub fn build(dir: &Path, blob: &[u8]) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let tables: Vec<PreloadTable> = load_byte_ngram(blob);
        let mut manifest = format!("{FORMAT}\nblob-len {}\nblob-hash {:016x}\n", blob.len(), blob_hash(blob));
        for (order, rows) in &tables {
            let cap = capacity_for(rows.len());
            let mask = cap - 1;
            let mut table = vec![0u8; cap * SLOT];
            let mut zero_row = None;
            for (&key, &(a, b)) in rows {
                if key == 0 {
                    zero_row = Some((a, b));
                    continue;
                }
                let mut i = (key as usize) & mask;
                loop {
                    let off = i * SLOT;
                    let here = u64::from_le_bytes(table[off..off + 8].try_into().expect("eight bytes"));
                    if here == 0 {
                        table[off..off + 8].copy_from_slice(&key.to_le_bytes());
                        table[off + 8..off + 12].copy_from_slice(&a.to_le_bytes());
                        table[off + 12..off + 16].copy_from_slice(&b.to_le_bytes());
                        break;
                    }
                    assert!(here != key, "the decoded table holds order {order} key {key:#x} once");
                    i = (i + 1) & mask;
                }
            }
            let path = table_path(dir, *order);
            let mut f = File::create(&path)?;
            f.write_all(&table)?;
            f.sync_all()?;
            manifest.push_str(&format!("order {order} cap {cap} rows {}", rows.len()));
            if let Some((a, b)) = zero_row {
                manifest.push_str(&format!(" zero {a} {b}"));
            }
            manifest.push('\n');
        }
        let mut f = File::create(dir.join(MANIFEST))?;
        f.write_all(manifest.as_bytes())?;
        f.sync_all()?;
        match Self::open(dir, blob)? {
            Some(prior) => Ok(prior),
            None => Err(bad(format!("the cache just written to {} does not describe this blob", dir.display()))),
        }
    }

    /// Map the cache in `dir` when it was built from `blob`. `Ok(None)` when
    /// `dir` has no manifest or its manifest names another blob; an error when
    /// the manifest or a table is not what the manifest says.
    ///
    /// The mapping is the one `unsafe` site, under the contract at the call:
    /// the crate denies `unsafe_code` and this function opts in, as the CUDA
    /// module does for its kernel launch.
    #[allow(unsafe_code)]
    pub fn open(dir: &Path, blob: &[u8]) -> io::Result<Option<Self>> {
        let manifest_path = dir.join(MANIFEST);
        let text = match fs::read_to_string(&manifest_path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut lines = text.lines();
        let mut head = |what: &str| -> io::Result<&str> {
            match lines.next() {
                Some(l) => Ok(l),
                None => Err(bad(format!("{} ends before its {what}", manifest_path.display()))),
            }
        };
        if head("format line")? != FORMAT {
            return Err(bad(format!("{} is not a {FORMAT} manifest", manifest_path.display())));
        }
        let len_line = head("blob length")?;
        let hash_line = head("blob hash")?;
        if !len_line.starts_with("blob-len ") || !hash_line.starts_with("blob-hash ") {
            return Err(bad(format!("{} lacks the blob length and hash", manifest_path.display())));
        }
        if len_line != format!("blob-len {}", blob.len()) || hash_line != format!("blob-hash {:016x}", blob_hash(blob)) {
            return Ok(None);
        }
        let mut orders = Vec::new();
        for line in lines.filter(|l| !l.trim().is_empty()) {
            let w: Vec<&str> = line.split_whitespace().collect();
            if w.first() != Some(&"order") || w.get(2) != Some(&"cap") || w.get(4) != Some(&"rows") {
                return Err(bad(format!("{}: unreadable line {line:?}", manifest_path.display())));
            }
            let order = field(&w, 1, "order", line)?;
            let cap = field(&w, 3, "cap", line)?;
            if !cap.is_power_of_two() {
                return Err(bad(format!("{}: order {order} cap {cap} is not a power of two", manifest_path.display())));
            }
            let zero_row = match w.get(6) {
                Some(&"zero") => Some((
                    narrow(field(&w, 7, "zero n0", line)?, "zero n0", line)?,
                    narrow(field(&w, 8, "zero n1", line)?, "zero n1", line)?,
                )),
                Some(other) => return Err(bad(format!("{}: unexpected {other:?} in {line:?}", manifest_path.display()))),
                None => None,
            };
            let path = table_path(dir, order);
            let f = File::open(&path)?;
            let len = f.metadata()?.len();
            if len != (cap * SLOT) as u64 {
                return Err(bad(format!("{} is {len} bytes, the manifest says {}", path.display(), cap * SLOT)));
            }
            // SAFETY: the file is read-only here and rewritten only by `build`,
            // which no caller runs while a mapping of the same cache is alive.
            let map = unsafe { Mmap::map(&f)? };
            orders.push(MappedOrder { order, map, cap, zero_row });
        }
        Ok(Some(Self { orders }))
    }

    /// The cache in `dir` for `blob`, built first when it is absent or was
    /// built from another blob. The flag says whether this call built it.
    pub fn open_or_build(dir: &Path, blob: &[u8]) -> io::Result<(Self, bool)> {
        match Self::open(dir, blob)? {
            Some(prior) => Ok((prior, false)),
            None => Ok((Self::build(dir, blob)?, true)),
        }
    }

    /// Bytes the mapped tables occupy on disk.
    #[must_use]
    pub fn bytes_on_disk(&self) -> usize {
        self.orders.iter().map(|o| o.cap * SLOT).sum()
    }

    /// The literal orders the cache holds.
    #[must_use]
    pub fn orders(&self) -> Vec<usize> {
        self.orders.iter().map(|o| o.order).collect()
    }
}

impl PriorLookup for MappedPrior {
    fn has_order(&self, order: usize) -> bool {
        self.orders.iter().any(|o| o.order == order)
    }

    fn get(&self, order: usize, key: u64) -> Option<(u32, u32)> {
        let t = self.orders.iter().find(|o| o.order == order)?;
        if key == 0 {
            return t.zero_row;
        }
        let mask = t.cap - 1;
        let mut i = (key as usize) & mask;
        loop {
            let off = i * SLOT;
            let here = u64::from_le_bytes(t.map[off..off + 8].try_into().expect("eight bytes"));
            if here == 0 {
                return None;
            }
            if here == key {
                let a = u32::from_le_bytes(t.map[off + 8..off + 12].try_into().expect("four bytes"));
                let b = u32::from_le_bytes(t.map[off + 12..off + 16].try_into().expect("four bytes"));
                return Some((a, b));
            }
            i = (i + 1) & mask;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seam::{byte_ngram_train, logistic_mix_bits, logistic_mix_bits_with, serialize_byte_ngram};

    const TEXT: &[u8] = b"It was the best of times, it was the worst of times, it was the age of wisdom, \
        it was the age of foolishness, it was the epoch of belief, it was the epoch of incredulity, \
        it was the season of Light, it was the season of Darkness, it was the spring of hope, \
        it was the winter of despair. 1859, 1859, 1859 and again 1859.";

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("trex/prior-cache-{name}-{}", std::process::id()))
    }

    #[test]
    fn the_mapped_prior_answers_every_row_the_decoded_one_holds() {
        let blob = serialize_byte_ngram(&byte_ngram_train(TEXT), 100_000);
        let heap = load_byte_ngram(&blob);
        let dir = scratch("rows");
        let (mapped, built) = MappedPrior::open_or_build(&dir, &blob).expect("the cache builds");
        assert!(built, "a fresh directory holds no cache");
        let mut rows = 0usize;
        for (order, table) in &heap {
            assert!(mapped.has_order(*order));
            for (&key, &counts) in table {
                assert_eq!(mapped.get(*order, key), Some(counts), "order {order} key {key:#x}");
                rows += 1;
            }
            assert_eq!(mapped.get(*order, 0x5eed_0000_dead_beef), None, "an absent key is absent");
        }
        assert!(rows > 100, "the sample trained {rows} rows, too few to test placement");
        assert!(!mapped.has_order(7) && mapped.get(7, 1).is_none(), "order 7 is not a model order");
        let (again, built_again) = MappedPrior::open_or_build(&dir, &blob).expect("the cache opens");
        assert!(!built_again, "the second call opened the cache it found");
        assert_eq!(again.orders(), mapped.orders());
        assert_eq!(again.bytes_on_disk(), mapped.bytes_on_disk());
        drop((mapped, again));
        fs::remove_dir_all(&dir).expect("the scratch cache is removed");
    }

    #[test]
    fn the_coder_codes_to_the_same_bits_against_either_form() {
        let blob = serialize_byte_ngram(&byte_ngram_train(TEXT), 100_000);
        let heap = load_byte_ngram(&blob);
        let dir = scratch("bits");
        let mapped = MappedPrior::build(&dir, &blob).expect("the cache builds");
        let input = b"it was the age of hope and the season of 1859; the worst of belief was the epoch of wisdom.";
        let from_heap = logistic_mix_bits(input, true, &[], Some(heap.as_slice()));
        let from_mapped = logistic_mix_bits_with(input, true, &[], Some(&mapped));
        let without = logistic_mix_bits(input, true, &[], None);
        assert!(from_heap == from_mapped, "heap {from_heap} bits, mapped {from_mapped}");
        assert!(from_heap != without, "the prior must change the bits, or the equality asserts nothing");
        drop(mapped);
        fs::remove_dir_all(&dir).expect("the scratch cache is removed");
    }

    #[test]
    fn a_cache_from_another_blob_is_not_opened() {
        let blob = serialize_byte_ngram(&byte_ngram_train(TEXT), 100_000);
        let other = serialize_byte_ngram(&byte_ngram_train(&TEXT[..200]), 100_000);
        let dir = scratch("stale");
        let first = MappedPrior::build(&dir, &blob).expect("the cache builds");
        drop(first);
        assert!(MappedPrior::open(&dir, &other).expect("the manifest reads").is_none());
        assert!(MappedPrior::open(&dir, &blob).expect("the manifest reads").is_some());
        assert!(MappedPrior::open(&scratch("absent"), &blob).expect("no directory reads as no cache").is_none());
        fs::remove_dir_all(&dir).expect("the scratch cache is removed");
    }
}
