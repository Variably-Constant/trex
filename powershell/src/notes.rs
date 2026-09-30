//! The scheduler's notes, a calibration it measured or a lever it read, kept
//! from whichever thread made them until a cmdlet writes them to its verbose
//! stream, the module's log, as its end begins; the console carries none of
//! them.

use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};

use pwrs::prelude::*;

/// One note kept, and the one kept before it.
struct Note {
    text: String,
    before: *mut Note,
}

/// The notes kept and not yet written, the newest first.
static KEPT: AtomicPtr<Note> = AtomicPtr::new(ptr::null_mut());

/// Keep `text`, a note the scheduler made on whichever thread it runs.
fn keep(text: &str) {
    let note = Box::into_raw(Box::new(Note { text: text.to_string(), before: ptr::null_mut() }));
    let mut newest = KEPT.load(Ordering::Relaxed);
    loop {
        // SAFETY: `note` is this thread's own until the exchange below
        // publishes it, so nothing else reads or writes it here.
        unsafe { (*note).before = newest };
        match KEPT.compare_exchange_weak(newest, note, Ordering::Release, Ordering::Relaxed) {
            Ok(_) => return,
            Err(now) => newest = now,
        }
    }
}

/// Every note kept since the last take, the oldest first.
fn take() -> Vec<String> {
    let mut at = KEPT.swap(ptr::null_mut(), Ordering::Acquire);
    let mut taken = Vec::new();
    while !at.is_null() {
        // SAFETY: the swap took the whole list out of `KEPT`, so no other
        // thread reaches these notes, each made by `Box::into_raw` in `keep`.
        let note = unsafe { Box::from_raw(at) };
        at = note.before;
        taken.push(note.text);
    }
    taken.reverse();
    taken
}

/// Write the notes kept so far to the verbose stream of `ps`.
pub(crate) fn write(ps: &Pipeline<'_>) -> PsResult<()> {
    for note in take() {
        ps.verbose(&note)?;
    }
    Ok(())
}

/// At import the scheduler's notes are kept for the verbose stream rather
/// than printed to the console.
#[on_import]
pub(crate) fn keep_notes() -> PsResult<()> {
    trex::notice::set_sink(Some(keep));
    Ok(())
}

/// At removal no cmdlet is left to write the notes, so the scheduler prints
/// them to the standard error again, and those still kept are let go.
#[on_remove]
pub(crate) fn release_notes() -> PsResult<()> {
    trex::notice::set_sink(None);
    drop(take());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Notes kept from several threads at once are all taken, each thread's
    /// in the order it kept them, and a take leaves none behind.
    #[test]
    fn notes_kept_from_many_threads_are_all_taken_in_order() {
        let threads: Vec<_> = (0..8)
            .map(|t| {
                std::thread::spawn(move || {
                    for i in 0..500 {
                        keep(&format!("{t}:{i}"));
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().expect("a keeping thread ends");
        }
        let taken = take();
        assert_eq!(taken.len(), 8 * 500);
        for t in 0..8 {
            let prefix = format!("{t}:");
            let own: Vec<usize> = taken
                .iter()
                .filter_map(|n| n.strip_prefix(prefix.as_str()))
                .map(|i| i.parse().expect("each note ends in its number"))
                .collect();
            assert_eq!(own, (0..500).collect::<Vec<usize>>(), "thread {t}");
        }
        assert!(take().is_empty());
    }
}
