use gitcomet_core::services::BlameLine;
use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

const BYTE_LIMIT: usize = super::super::OBJECT_CACHE_BYTES;
const ATTRIBUTE_BYTE_LIMIT: usize = 64 * 1024;
const ENTRY_LIMIT: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct WholeFileOptions {
    pub(super) algorithm: gix::diff::blob::Algorithm,
    pub(super) rewrites: Option<gix::diff::Rewrites>,
}

impl WholeFileOptions {
    pub(super) fn gix(self) -> gix::repository::blame_file::Options {
        gix::repository::blame_file::Options {
            diff_algorithm: Some(self.algorithm),
            rewrites: self.rewrites,
            ranges: Default::default(),
            since: None,
        }
    }
}

enum AttributeIndex {
    Index(Weak<gix::fs::FileSnapshot<gix::index::File>>),
    HeadTree(gix::ObjectId),
}

impl PartialEq for AttributeIndex {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Index(a), Self::Index(b)) => a.ptr_eq(b),
            (Self::HeadTree(a), Self::HeadTree(b)) => a == b,
            _ => false,
        }
    }
}

#[derive(PartialEq)]
pub(super) struct Key {
    generation: u64,
    suspect: gix::ObjectId,
    path: OsString,
    options: WholeFileOptions,
    index: AttributeIndex,
    external_attributes: Vec<Vec<u8>>,
}

impl Key {
    pub(super) fn read(
        repo: &gix::Repository,
        generation: u64,
        suspect: gix::ObjectId,
        path: &Path,
        options: WholeFileOptions,
    ) -> Option<Self> {
        if repo.is_shallow().ok()? {
            return None;
        }
        let index = match repo.try_index().ok()? {
            Some(index) => AttributeIndex::Index(Arc::downgrade(&index)),
            None => AttributeIndex::HeadTree(crate::refs::head_tree_id_or_empty(repo).ok()?),
        };
        let mut files = repo.attribute_global_paths().ok()?;
        files.push(repo.common_dir().join("info/attributes"));
        let mut external_attributes = Vec::new();
        let mut remaining = ATTRIBUTE_BYTE_LIMIT;
        for path in files {
            let mut bytes = Vec::new();
            match std::fs::File::open(path) {
                Ok(file) => {
                    file.take(remaining as u64 + 1)
                        .read_to_end(&mut bytes)
                        .ok()?;
                    remaining = remaining.checked_sub(bytes.len())?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return None,
            }
            external_attributes.push(bytes);
        }
        Some(Self {
            generation,
            suspect,
            path: path.as_os_str().to_owned(),
            options,
            index,
            external_attributes,
        })
    }

    fn bytes(&self) -> usize {
        self.path.capacity()
            + self.external_attributes.capacity() * size_of::<Vec<u8>>()
            + self
                .external_attributes
                .iter()
                .map(Vec::capacity)
                .sum::<usize>()
    }
}

pub(super) struct Entry {
    key: Key,
    lines: Arc<Vec<BlameLine>>,
    bytes: usize,
}

#[derive(Default)]
pub(in crate::repo) struct Cache {
    entries: VecDeque<Entry>,
    bytes: usize,
}

impl Cache {
    pub(super) fn get(&mut self, key: &Key) -> Option<Arc<Vec<BlameLine>>> {
        let at = self.entries.iter().position(|entry| entry.key == *key)?;
        let entry = self.entries.remove(at)?;
        let lines = Arc::clone(&entry.lines);
        self.entries.push_back(entry);
        Some(lines)
    }

    pub(super) fn insert(&mut self, entry: Entry) -> Vec<Entry> {
        if self.entries.iter().any(|old| old.key == entry.key) {
            return vec![entry];
        }
        let bytes = entry.bytes;
        let mut released = Vec::new();
        while self.bytes + bytes > BYTE_LIMIT || self.entries.len() >= ENTRY_LIMIT {
            let oldest = self.entries.pop_front().expect("nonempty blame cache");
            self.bytes -= oldest.bytes;
            released.push(oldest);
        }
        self.entries.push_back(entry);
        self.bytes += bytes;
        released
    }
}

impl Entry {
    pub(super) fn new(key: Key, lines: &Arc<Vec<BlameLine>>) -> Option<Self> {
        let mut bytes = size_of::<Self>() + key.bytes() + lines.capacity() * size_of::<BlameLine>();
        if bytes > BYTE_LIMIT {
            return None;
        }
        for line in lines.iter() {
            // Charge even shared metadata to each row. Admission is conservative and bounded.
            bytes += line.line.capacity()
                + line.commit_id.len()
                + line.author.len()
                + line.summary.len()
                + line.body.as_deref().map_or(0, str::len)
                + line.prior_commit.as_deref().map_or(0, str::len)
                + line.source_path.as_ref().map_or(0, PathBuf::capacity)
                + 10 * size_of::<usize>();
            if bytes > BYTE_LIMIT {
                return None;
            }
        }
        Some(Self {
            key,
            lines: Arc::clone(lines),
            bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(id: usize) -> Key {
        let oid = gix::hash::Kind::Sha1.null();
        Key {
            generation: 1,
            suspect: oid,
            path: format!("file-{id}").into(),
            options: WholeFileOptions {
                algorithm: Default::default(),
                rewrites: None,
            },
            index: AttributeIndex::HeadTree(oid),
            external_attributes: Vec::new(),
        }
    }

    #[test]
    fn attribution_is_bounded_by_bytes_and_recent_keys() {
        let mut cache = Cache::default();
        let empty = Arc::new(Vec::new());
        for id in 0..ENTRY_LIMIT {
            cache.insert(Entry::new(key(id), &empty).unwrap());
        }
        assert!(cache.get(&key(0)).is_some());
        cache.insert(Entry::new(key(ENTRY_LIMIT), &empty).unwrap());
        assert!(cache.get(&key(0)).is_some());
        assert!(cache.get(&key(1)).is_none());

        let mut changed = key(0);
        changed.options.rewrites = Some(Default::default());
        assert!(cache.get(&changed).is_none());
        changed.options = key(0).options;
        changed.options.algorithm = gix::diff::blob::Algorithm::MyersMinimal;
        assert!(cache.get(&changed).is_none());

        let huge = Arc::new(vec![BlameLine {
            commit_id: "".into(),
            author: "".into(),
            author_time_unix: None,
            summary: "".into(),
            body: None,
            line: "x".repeat(BYTE_LIMIT),
            prior_exists: false,
            source_path: None,
            prior_commit: None,
        }]);
        assert!(Entry::new(key(10), &huge).is_none());
        assert_eq!(cache.entries.len(), ENTRY_LIMIT);
    }
}
