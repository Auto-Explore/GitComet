use gpui::SharedString;
use rustc_hash::FxHasher;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// A stamp younger than this cannot be trusted: same length in the same mtime
/// tick may still be different bytes (git's racy-file rule).
const DISK_STAMP_RACY_WINDOW: Duration = Duration::from_secs(2);

/// Saves auto-save can have in flight at once is one or two; this only caps
/// a pathological queue.
const MAX_KNOWN_BYTES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::view) struct DiskStamp {
    len: u64,
    modified: Option<SystemTime>,
    /// Inode and change time: mtime can be set back (`cp -p`, `rsync -t`),
    /// ctime cannot, and a rename-over swaps the inode.
    #[cfg(unix)]
    change: (u64, i64, i64),
    /// False while `modified` was within the racy window of when it was read.
    trusted: bool,
}

impl DiskStamp {
    fn same_file_state(self, other: DiskStamp) -> bool {
        #[cfg(unix)]
        if self.change != other.change {
            return false;
        }
        self.len == other.len && self.modified == other.modified
    }
}

pub(in crate::view) fn disk_stamp(meta: &std::fs::Metadata) -> DiskStamp {
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;

    let modified = meta.modified().ok();
    let trusted = modified.is_some_and(|modified| {
        SystemTime::now()
            .duration_since(modified)
            .is_ok_and(|age| age > DISK_STAMP_RACY_WINDOW)
    });
    DiskStamp {
        len: meta.len(),
        modified,
        #[cfg(unix)]
        change: (meta.ino(), meta.ctime(), meta.ctime_nsec()),
        trusted,
    }
}

/// Bytes a surface knows to be its own.
#[derive(Clone, Debug, PartialEq, Eq)]
enum KnownBytes {
    /// Seen on disk, by hash.
    Seen(u64),
    /// A save dispatched and not yet seen on disk, by its bytes. Compared on the
    /// check's thread, so saving never hashes the file on the UI thread.
    Pending(PendingWrite),
}

/// Bytes a save is writing: the buffer's own text when the file is UTF-8,
/// otherwise its encoding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::view) enum PendingWrite {
    Text(SharedString),
    Encoded(Arc<[u8]>),
}

impl PendingWrite {
    fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Text(text) => text.as_bytes(),
            Self::Encoded(bytes) => bytes,
        }
    }
}

impl From<SharedString> for PendingWrite {
    fn from(text: SharedString) -> Self {
        Self::Text(text)
    }
}

impl From<&'static str> for PendingWrite {
    fn from(text: &'static str) -> Self {
        Self::Text(text.into())
    }
}

impl KnownBytes {
    fn matches(&self, bytes: &[u8], hash: u64) -> bool {
        match self {
            Self::Seen(seen) => *seen == hash,
            Self::Pending(write) => write.as_bytes() == bytes,
        }
    }
}

/// What a surface agrees with on disk.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::view) struct DiskIdentity {
    stamp: Option<DiskStamp>,
    /// In the order they are (or will be) on disk: the content last seen
    /// there first, then saves dispatched since. Empty when the file was too
    /// large to hash.
    known: Vec<KnownBytes>,
    /// The file was gone at the last look.
    pub(in crate::view) missing: bool,
}

impl DiskIdentity {
    pub(in crate::view) fn missing() -> Self {
        Self {
            missing: true,
            ..Self::default()
        }
    }
    pub(in crate::view) fn loaded(stamp: DiskStamp, hash: Option<u64>) -> Self {
        Self {
            stamp: Some(stamp),
            known: hash.map(KnownBytes::Seen).into_iter().collect(),
            missing: false,
        }
    }

    /// A write was dispatched but has not necessarily landed, so both the
    /// bytes before it and the bytes it writes are ours for now. `text` is the
    /// handle the save already built; nothing is copied or hashed here.
    pub(in crate::view) fn note_pending_write(&mut self, write: PendingWrite) {
        let pending = KnownBytes::Pending(write);
        if self.known.last() != Some(&pending) {
            self.known.push(pending);
            if self.known.len() > MAX_KNOWN_BYTES {
                self.known.remove(0);
            }
        }
        if let Some(stamp) = self.stamp.as_mut() {
            stamp.trusted = false;
        }
    }

    /// The disk was seen holding entry `ix` (hashing to `hash`): every write
    /// queued before it has landed, so what it replaced is no longer ours.
    /// Without this a program writing back the version loaded before a save
    /// would pass as known.
    pub(in crate::view) fn confirm(&mut self, stamp: DiskStamp, matched: Option<(usize, u64)>) {
        self.stamp = Some(stamp);
        self.missing = false;
        if let Some((ix, hash)) = matched
            && ix < self.known.len()
        {
            self.known.drain(..ix);
            self.known[0] = KnownBytes::Seen(hash);
        }
    }

    /// Take `seen` as what is on disk, keeping any save still on its way:
    /// that write will land on top of it and is ours.
    pub(in crate::view) fn adopt(&mut self, seen: DiskIdentity) {
        let pending: Vec<KnownBytes> = self
            .known
            .drain(..)
            .filter(|known| matches!(known, KnownBytes::Pending(_)))
            .collect();
        *self = seen;
        self.known.extend(pending);
        self.known.truncate(MAX_KNOWN_BYTES);
    }

    pub(in crate::view) fn same_disk_state(&self, other: &Self) -> bool {
        self.missing == other.missing
            && self.known == other.known
            && match (self.stamp, other.stamp) {
                (Some(a), Some(b)) => a.same_file_state(b),
                (None, None) => true,
                _ => false,
            }
    }
}

/// One `write` over the raw bytes. Not `file_editor_text_fingerprint`: that one
/// folds rope chunks and a length prefix, and cannot agree with a flat read.
pub(in crate::view) fn disk_content_hash(bytes: &[u8]) -> u64 {
    use std::hash::Hasher;

    let mut hasher = FxHasher::default();
    hasher.write(bytes);
    hasher.finish()
}

#[derive(Debug)]
pub(in crate::view) enum DiskCheckOutcome {
    Unchanged,
    /// Known bytes under a new stamp (our own write landed, or a touch):
    /// which entry matched, and the disk's hash.
    Same {
        fresh: DiskStamp,
        matched: Option<(usize, u64)>,
    },
    Changed {
        fresh: DiskStamp,
        hash: Option<u64>,
    },
    Missing,
}

/// Compare the file at `path` with `known`. Reads the bytes only when the
/// stamp cannot settle it, and only up to `hash_limit`.
pub(in crate::view) fn check_disk_identity(
    path: &Path,
    known: &DiskIdentity,
    hash_limit: u64,
) -> DiskCheckOutcome {
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return if known.missing {
                DiskCheckOutcome::Unchanged
            } else {
                DiskCheckOutcome::Missing
            };
        }
        // A transient error is not a change worth nagging about.
        Err(_) => return DiskCheckOutcome::Unchanged,
    };
    if meta.is_dir() {
        return DiskCheckOutcome::Unchanged;
    }
    let fresh = disk_stamp(&meta);
    let stamp_matches = known
        .stamp
        .is_some_and(|stamp| stamp.same_file_state(fresh));
    if stamp_matches && known.stamp.is_some_and(|stamp| stamp.trusted) {
        return DiskCheckOutcome::Unchanged;
    }
    if fresh.len <= hash_limit && !known.known.is_empty() {
        return match std::fs::read(path) {
            Ok(bytes) => {
                let hash = disk_content_hash(&bytes);
                match known.known.iter().position(|k| k.matches(&bytes, hash)) {
                    Some(ix) => DiskCheckOutcome::Same {
                        fresh,
                        matched: Some((ix, hash)),
                    },
                    None => DiskCheckOutcome::Changed {
                        fresh,
                        hash: Some(hash),
                    },
                }
            }
            Err(_) => DiskCheckOutcome::Unchanged,
        };
    }
    if stamp_matches {
        DiskCheckOutcome::Same {
            fresh,
            matched: None,
        }
    } else {
        DiskCheckOutcome::Changed { fresh, hash: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(len: u64) -> DiskStamp {
        DiskStamp {
            len,
            modified: None,
            #[cfg(unix)]
            change: (0, 0, 0),
            trusted: false,
        }
    }

    fn pending(text: &str) -> KnownBytes {
        KnownBytes::Pending(PendingWrite::Text(SharedString::from(text.to_string())))
    }

    #[test]
    fn confirming_a_write_forgets_what_it_replaced_but_keeps_later_writes() {
        let mut identity = DiskIdentity::loaded(stamp(1), Some(10));
        identity.note_pending_write("two".into());
        identity.note_pending_write("three".into());
        assert_eq!(
            identity.known,
            vec![KnownBytes::Seen(10), pending("two"), pending("three")]
        );

        // Seen before any write landed: nothing is history yet.
        identity.confirm(stamp(1), Some((0, 10)));
        assert_eq!(identity.known.len(), 3);

        // The first write landed; the loaded bytes are now someone else's.
        identity.confirm(stamp(2), Some((1, 20)));
        assert_eq!(identity.known, vec![KnownBytes::Seen(20), pending("three")]);

        identity.confirm(stamp(3), Some((1, 30)));
        assert_eq!(identity.known, vec![KnownBytes::Seen(30)]);
    }

    #[test]
    fn a_pending_write_matches_its_own_bytes_off_the_ui_thread() {
        let known = pending("fn main() {}\n");
        assert!(known.matches(b"fn main() {}\n", 0));
        assert!(!known.matches(b"fn main() { }\n", 0));
        assert!(KnownBytes::Seen(7).matches(b"anything", 7));
    }

    #[test]
    fn an_encoded_save_is_recognized_by_its_bytes_not_its_text() {
        let known = KnownBytes::Pending(PendingWrite::Encoded(Arc::from(&b"caf\xe9\n"[..])));
        assert!(known.matches(b"caf\xe9\n", 0));
        assert!(!known.matches("café\n".as_bytes(), 0));
    }

    #[test]
    fn adopting_what_the_notice_saw_keeps_saves_still_on_their_way() {
        let mut identity = DiskIdentity::loaded(stamp(1), Some(10));
        identity.note_pending_write("mine".into());
        identity.adopt(DiskIdentity::loaded(stamp(2), Some(99)));
        assert_eq!(identity.known, vec![KnownBytes::Seen(99), pending("mine")]);
    }

    #[test]
    fn pending_writes_are_capped() {
        let mut identity = DiskIdentity::loaded(stamp(1), Some(0));
        for ix in 1..=20 {
            identity.note_pending_write(SharedString::from(ix.to_string()).into());
        }
        assert_eq!(identity.known.len(), MAX_KNOWN_BYTES);
        assert_eq!(identity.known.last(), Some(&pending("20")));
    }
}
