//! Following files as they grow: the bytes appended to each as they arrive,
//! and a note where one was truncated, replaced under its name, or removed.
//!
//! Each file is followed by name, so a log rotated out from under a reader
//! is picked up again at the file that now bears the name, read from its
//! start. The operating system's change notifications (inotify,
//! ReadDirectoryChangesW, FSEvents or kqueue, through the `notify` crate)
//! wake a follower on a change to a watched file's directory; a second
//! passing with no notification is checked all the same, as GNU `tail -f`
//! checks every second, so a change a notifier misses is late by at most that.

use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

/// How long a follower waits for a notification before it looks at its files
/// anyway: GNU `tail -f`'s own interval.
const CHECK_EVERY: Duration = Duration::from_secs(1);

/// What following one file brought.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Followed {
    /// Bytes appended to the file, and where they begin in it.
    Appended { bytes: Vec<u8>, at: usize },
    /// The file shrank below what had been read of it; it is read again from
    /// its start.
    Truncated,
    /// Another file now has the name; it is read from its start.
    Replaced,
    /// The name no longer names a file; following waits for one to appear
    /// under it.
    Gone,
}

/// What tells two files under one name apart: the device and inode on Unix,
/// the volume serial number and file index on Windows. NTFS gives a file
/// created under a name removed moments before the old file's creation time,
/// so a creation time cannot tell them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity(u64, u64);

/// The identity of the file `path` names now.
fn identity_of(path: &Path) -> io::Result<Identity> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::metadata(path)?;
        Ok(Identity(meta.dev(), meta.ino()))
    }
    #[cfg(windows)]
    {
        let file = std::fs::File::open(path)?;
        let info = winapi_util::file::information(&file)?;
        Ok(Identity(info.volume_serial_number(), info.file_index()))
    }
    #[cfg(not(any(unix, windows)))]
    {
        compile_error!("following a file reads its identity, which trex reads on Unix and Windows")
    }
}

/// One file a follower reads: its name, the file open under it where there
/// is one, which file that is, and how much of it has been read.
struct Followee {
    path: PathBuf,
    file: Option<std::fs::File>,
    identity: Option<Identity>,
    offset: usize,
}

impl Followee {
    /// What changed in the file since it was last looked at, if anything.
    fn look(&mut self) -> io::Result<Option<Followed>> {
        let now = match identity_of(&self.path) {
            Ok(id) => id,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if self.file.take().is_some() {
                    self.identity = None;
                    self.offset = 0;
                    return Ok(Some(Followed::Gone));
                }
                return Ok(None);
            }
            Err(e) => return Err(e),
        };
        if self.identity != Some(now) || self.file.is_none() {
            let known = self.identity.is_some();
            self.file = Some(std::fs::File::open(&self.path)?);
            self.identity = Some(now);
            self.offset = 0;
            if known {
                return Ok(Some(Followed::Replaced));
            }
        }
        let Some(file) = self.file.as_mut() else {
            return Ok(None);
        };
        let len = usize::try_from(file.metadata()?.len())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{}: {e}", self.path.display())))?;
        if len < self.offset {
            self.offset = 0;
            return Ok(Some(Followed::Truncated));
        }
        if len == self.offset {
            return Ok(None);
        }
        file.seek(SeekFrom::Start(self.offset as u64))?;
        let mut bytes = Vec::with_capacity(len - self.offset);
        file.by_ref().take((len - self.offset) as u64).read_to_end(&mut bytes)?;
        if bytes.is_empty() {
            return Ok(None);
        }
        let at = self.offset;
        self.offset += bytes.len();
        Ok(Some(Followed::Appended { bytes, at }))
    }
}

/// Files followed as they grow, woken by the operating system's change
/// notifications.
pub struct Follower {
    files: Vec<Followee>,
    /// The notifier and what it sends, kept alive while following; `None`
    /// where no notifier could be made, in which case every file is looked at
    /// each second.
    notified: Option<(notify::RecommendedWatcher, Receiver<notify::Result<notify::Event>>)>,
    /// Why no notifier could be made, for a caller to say.
    unnotified: Option<String>,
    /// Changes found and not yet handed out, as file index and change.
    pending: std::collections::VecDeque<(usize, Followed)>,
}

impl Follower {
    /// Follow each of `files` from the offset beside it: where a head or tail
    /// of it was read to, or 0 to follow it from its start. A file not there
    /// yet is followed from when it appears.
    ///
    /// # Errors
    ///
    /// A file that is there cannot be opened or measured.
    pub fn new(files: &[(PathBuf, usize)]) -> io::Result<Follower> {
        let mut followees = Vec::with_capacity(files.len());
        for (path, offset) in files {
            let (file, identity) = match identity_of(path) {
                Ok(id) => (Some(std::fs::File::open(path)?), Some(id)),
                Err(e) if e.kind() == io::ErrorKind::NotFound => (None, None),
                Err(e) => return Err(e),
            };
            followees.push(Followee { path: path.clone(), file, identity, offset: *offset });
        }
        let (notified, unnotified) = match Self::notifier(&followees) {
            Ok(pair) => (Some(pair), None),
            Err(e) => (None, Some(e.to_string())),
        };
        Ok(Follower { files: followees, notified, unnotified, pending: std::collections::VecDeque::new() })
    }

    /// A notifier watching the directory of every file, so a file created or
    /// renamed under a followed name is seen as well as one written to.
    fn notifier(
        files: &[Followee],
    ) -> notify::Result<(notify::RecommendedWatcher, Receiver<notify::Result<notify::Event>>)> {
        use notify::Watcher;
        let (tx, rx) = channel();
        let mut watcher = notify::recommended_watcher(tx)?;
        let mut watched: Vec<PathBuf> = Vec::new();
        for f in files {
            let dir = match f.path.parent() {
                Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
                Some(_) | None => PathBuf::from("."),
            };
            if !watched.contains(&dir) {
                watcher.watch(&dir, notify::RecursiveMode::NonRecursive)?;
                watched.push(dir);
            }
        }
        Ok((watcher, rx))
    }

    /// Why this follower has no change notifier and looks at its files each
    /// second instead, where that is so.
    #[must_use]
    pub fn unnotified(&self) -> Option<&str> {
        self.unnotified.as_deref()
    }

    /// The name of file `index`, as it was given.
    #[must_use]
    pub fn path(&self, index: usize) -> &Path {
        &self.files[index].path
    }

    /// The next change to a followed file, as the index of the file and what
    /// changed, waiting for one as long as it takes.
    ///
    /// # Errors
    ///
    /// A followed file cannot be read, or the notifier reports a failure.
    pub fn wait(&mut self) -> io::Result<(usize, Followed)> {
        loop {
            if let Some(change) = self.poll()? {
                return Ok(change);
            }
        }
    }

    /// The next change to a followed file, where one comes within a second:
    /// the files are looked at, and where none has changed, a notification is
    /// waited for until the next look is due. A caller that must stop when
    /// asked, as a PowerShell cmdlet must, polls rather than waits.
    ///
    /// # Errors
    ///
    /// A followed file cannot be read, or the notifier reports a failure.
    pub fn poll(&mut self) -> io::Result<Option<(usize, Followed)>> {
        if let Some(change) = self.pending.pop_front() {
            return Ok(Some(change));
        }
        self.look_at_all()?;
        if let Some(change) = self.pending.pop_front() {
            return Ok(Some(change));
        }
        match &self.notified {
            Some((_, rx)) => match rx.recv_timeout(CHECK_EVERY) {
                Ok(Ok(_event)) => {}
                Ok(Err(e)) => return Err(io::Error::other(e)),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(io::Error::other("the change notifier stopped"));
                }
            },
            None => std::thread::sleep(CHECK_EVERY),
        }
        self.look_at_all()?;
        Ok(self.pending.pop_front())
    }

    /// Look at every file, queueing what changed in each.
    fn look_at_all(&mut self) -> io::Result<()> {
        for (k, f) in self.files.iter_mut().enumerate() {
            // A file whose name was replaced is looked at again at once, so
            // the new file's first bytes follow the note that it is new.
            while let Some(change) =
                f.look().map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", f.path.display())))?
            {
                let more = matches!(change, Followed::Replaced | Followed::Truncated);
                self.pending.push_back((k, change));
                if !more {
                    break;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// What a follower sees of a file appended to, truncated, and replaced
    /// under its name, in the order it happened.
    #[test]
    fn a_follower_sees_appends_truncation_and_replacement_in_order() {
        let dir = std::env::temp_dir().join(format!("trex-follow-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create the directory");
        let path = dir.join("app.log");
        std::fs::write(&path, b"one\n").expect("write the file");
        let mut follower = Follower::new(&[(path.clone(), 4)]).expect("follow the file");

        let mut file = std::fs::OpenOptions::new().append(true).open(&path).expect("open to append");
        file.write_all(b"two\n").expect("append");
        drop(file);
        assert_eq!(follower.wait().expect("a change"), (0, Followed::Appended { bytes: b"two\n".to_vec(), at: 4 }));

        std::fs::write(&path, b"x\n").expect("truncate and write");
        let (k, first) = follower.wait().expect("a change");
        assert_eq!((k, first), (0, Followed::Truncated));
        assert_eq!(follower.wait().expect("a change"), (0, Followed::Appended { bytes: b"x\n".to_vec(), at: 0 }));

        let rotated = dir.join("app.log.1");
        std::fs::rename(&path, &rotated).expect("rotate");
        std::fs::write(&path, b"new\n").expect("write the new file");
        let mut seen = Vec::new();
        while seen.len() < 2 {
            seen.push(follower.wait().expect("a change").1);
        }
        // Where the rename is seen before the new file, the name was gone for
        // a moment; either way the new file's bytes follow the note.
        let appended = Followed::Appended { bytes: b"new\n".to_vec(), at: 0 };
        let gone_then_new = seen[0] == Followed::Gone;
        if gone_then_new {
            assert_eq!(seen[1], appended);
        } else {
            assert_eq!(seen, vec![Followed::Replaced, appended]);
        }
        std::fs::remove_dir_all(&dir).expect("remove the directory");
    }
}
