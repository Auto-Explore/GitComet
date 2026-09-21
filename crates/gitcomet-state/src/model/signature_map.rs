use gitcomet_core::domain::{CommitId, CommitSignature};
use rustc_hash::{FxHashMap, FxHasher};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

const BUCKETS: usize = 64;

/// A snapshot-friendly signature map. Updating a small batch copies only the
/// affected buckets; published snapshots continue sharing every other bucket.
#[derive(Clone, Debug)]
pub struct CommitSignatureMap {
    buckets: [Option<Arc<FxHashMap<CommitId, CommitSignature>>>; BUCKETS],
    len: usize,
}

impl Default for CommitSignatureMap {
    fn default() -> Self {
        Self {
            buckets: std::array::from_fn(|_| None),
            len: 0,
        }
    }
}

impl CommitSignatureMap {
    fn bucket(id: &CommitId) -> usize {
        let mut hash = FxHasher::default();
        id.hash(&mut hash);
        hash.finish() as usize % BUCKETS
    }

    pub fn get(&self, id: &CommitId) -> Option<&CommitSignature> {
        self.buckets[Self::bucket(id)].as_ref()?.get(id)
    }

    pub fn contains_key(&self, id: &CommitId) -> bool {
        self.get(id).is_some()
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(crate) fn insert(&mut self, id: CommitId, signature: CommitSignature) {
        let bucket = self.buckets[Self::bucket(&id)].get_or_insert_with(Arc::default);
        if Arc::make_mut(bucket).insert(id, signature).is_none() {
            self.len += 1;
        }
    }

    pub(crate) fn remove(&mut self, id: &CommitId) {
        if let Some(bucket) = &mut self.buckets[Self::bucket(id)]
            && bucket.contains_key(id)
        {
            Arc::make_mut(bucket).remove(id);
            self.len -= 1;
        }
    }
}

impl FromIterator<(CommitId, CommitSignature)> for CommitSignatureMap {
    fn from_iter<T: IntoIterator<Item = (CommitId, CommitSignature)>>(iter: T) -> Self {
        let mut map = Self::default();
        for (id, signature) in iter {
            map.insert(id, signature);
        }
        map
    }
}

impl std::ops::Index<&CommitId> for CommitSignatureMap {
    type Output = CommitSignature;
    fn index(&self, id: &CommitId) -> &Self::Output {
        self.get(id).expect("signature key not found")
    }
}
