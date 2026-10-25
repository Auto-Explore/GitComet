//! Optional decoded retention is bounded globally; caller-pinned blocks are
//! registered weakly and remain reusable even after their retention is evicted.
use super::GixRepo;
use gitcomet_core::history_index::{
    HISTORY_ROW_CACHE_LIMIT, HistoryIndex, HistoryIndexHandle, HistoryRange,
};
use gitcomet_core::services::{CancellationToken, Result};
use std::collections::VecDeque;
use std::ops::Range;
use std::sync::{Arc, Mutex, Weak};

const BYTE_LIMIT: usize = 32 * 1024 * 1024;

struct Entry {
    common: u64,
    generation: u64,
    index: Weak<HistoryIndex>,
    rows: Range<usize>,
    block: Weak<HistoryRange>,
    retained: Option<Arc<HistoryRange>>,
    bytes: usize,
}

#[derive(Default)]
struct Cache {
    entries: VecDeque<Entry>,
}
static CACHE: Mutex<Cache> = Mutex::new(Cache {
    entries: VecDeque::new(),
});

/// Conservative allocation accounting: charge shared strings per occurrence,
/// including Arc headers, so interning cannot make retention exceed the budget.
fn block_bytes(block: &HistoryRange) -> usize {
    let mut bytes = std::mem::size_of::<HistoryRange>()
        + 2 * std::mem::size_of::<usize>()
        + block.commits.capacity() * std::mem::size_of::<gitcomet_core::domain::Commit>()
        + block.snapshot.0.len()
        + 2 * std::mem::size_of::<usize>();
    for commit in &block.commits {
        bytes = bytes.saturating_add(
            commit.id.as_ref().len()
                + commit.summary.len()
                + commit.author.len()
                + 6 * std::mem::size_of::<usize>()
                + commit.parent_ids.capacity()
                    * std::mem::size_of::<gitcomet_core::domain::CommitId>(),
        );
        for parent in &commit.parent_ids {
            bytes = bytes.saturating_add(parent.as_ref().len() + 2 * std::mem::size_of::<usize>());
        }
    }
    bytes
}

impl Cache {
    fn prune(&mut self) {
        for entry in &mut self.entries {
            if entry.index.strong_count() == 0 {
                entry.retained = None;
            }
        }
        self.entries.retain(|entry| entry.block.strong_count() != 0);
    }

    fn get(
        &mut self,
        generation: u64,
        index: &HistoryIndexHandle,
        rows: &Range<usize>,
    ) -> Option<Arc<HistoryRange>> {
        self.prune();
        let at = self.entries.iter().position(|entry| {
            entry.generation == generation
                && entry.index.ptr_eq(&Arc::downgrade(index))
                && &entry.rows == rows
        })?;
        let entry = self.entries.remove(at)?;
        let block = entry.block.upgrade();
        self.entries.push_back(entry);
        block
    }

    fn insert(
        &mut self,
        common: u64,
        generation: u64,
        index: &HistoryIndexHandle,
        block: &Arc<HistoryRange>,
    ) {
        self.prune();
        let bytes = block_bytes(block);
        self.entries.push_back(Entry {
            common,
            generation,
            index: Arc::downgrade(index),
            rows: block.range(),
            block: Arc::downgrade(block),
            retained: Some(block.clone()),
            bytes,
        });
        let mut total: usize = self
            .entries
            .iter()
            .filter(|e| e.retained.is_some())
            .map(|e| e.bytes)
            .sum();
        let mut rows: usize = self
            .entries
            .iter()
            .filter(|e| e.common == common && e.retained.is_some())
            .map(|e| e.rows.len())
            .sum();
        for entry in &mut self.entries {
            if (total > BYTE_LIMIT || (entry.common == common && rows > HISTORY_ROW_CACHE_LIMIT))
                && entry.retained.take().is_some()
            {
                total -= entry.bytes;
                if entry.common == common {
                    rows -= entry.rows.len();
                }
            }
        }
        self.prune();
    }
}

pub(super) fn invalidate(common: u64) {
    let mut cache = CACHE.lock().expect("shared ranges");
    for entry in &mut cache.entries {
        if entry.common == common {
            entry.retained = None;
        }
    }
    cache.prune();
}

/// (optional retained bytes, caller-pinned bytes, optional retained rows).
/// Pinned bytes overlap retained bytes while both cache and callers own a block.
pub(crate) fn memory() -> (usize, usize, usize) {
    let mut cache = CACHE.lock().expect("shared ranges");
    cache.prune();
    cache
        .entries
        .iter()
        .fold((0, 0, 0), |(retained, pinned, rows), entry| {
            let kept = usize::from(entry.retained.is_some());
            (
                retained + kept * entry.bytes,
                pinned + usize::from(entry.block.strong_count() > kept) * entry.bytes,
                rows + kept * entry.rows.len(),
            )
        })
}

impl GixRepo {
    pub(super) fn validate_history_store(
        &self,
        index: &HistoryIndexHandle,
        shared: &super::shared::SharedStore,
    ) -> Result<()> {
        if index.snapshot.0.starts_with("gix-history:") && !shared.owns_index(index) {
            return Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend(
                    "History object interpretation changed; refresh history".into(),
                ),
            ));
        }
        Ok(())
    }

    pub(super) fn read_shared_history_range(
        &self,
        index: &HistoryIndexHandle,
        rows: Range<usize>,
        cancellation: &CancellationToken,
    ) -> Result<Arc<HistoryRange>> {
        cancellation.check_cancelled()?;
        let (_, shared) = self.fresh_history_store()?;
        self.validate_history_store(index, &shared)?;
        // Decode once for concurrent callers, without holding the global cache
        // lock through I/O. Cancellation remains responsive while waiting.
        let _decode = loop {
            cancellation.check_cancelled()?;
            match shared.decode.try_lock() {
                Ok(guard) => break guard,
                Err(std::sync::TryLockError::WouldBlock) => {
                    std::thread::sleep(std::time::Duration::from_millis(5))
                }
                Err(std::sync::TryLockError::Poisoned(error)) => break error.into_inner(),
            }
        };
        if let Some(block) = CACHE
            .lock()
            .expect("shared ranges")
            .get(shared.id, index, &rows)
        {
            return Ok(block);
        }
        let block = Arc::new(self.read_history_range_impl(index, rows, cancellation)?);
        CACHE
            .lock()
            .expect("shared ranges")
            .insert(shared.common.id, shared.id, index, &block);
        cancellation.check_cancelled()?;
        Ok(block)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{Commit, CommitId, HistoryMode};
    use gitcomet_core::history_index::HistoryIndexBuilder;
    use gitcomet_core::services::HistorySnapshot;

    #[test]
    fn retention_obeys_process_byte_and_common_repository_row_limits() {
        let index = HistoryIndexBuilder::new(
            HistorySnapshot("budget".into()),
            HistoryMode::AllBranches,
            20,
        )
        .unwrap()
        .finish(&CancellationToken::new())
        .unwrap();
        let commit = Commit {
            id: CommitId("a".repeat(40).into()),
            parent_ids: Default::default(),
            author: "a".into(),
            summary: "text".into(),
            time: std::time::UNIX_EPOCH,
        };
        let mut cache = Cache::default();
        let mut pinned = Vec::new();
        for generation in 0..3 {
            for start in (0..8192).step_by(256) {
                let block = Arc::new(HistoryRange {
                    snapshot: index.snapshot.clone(),
                    start,
                    commits: vec![commit.clone(); 256],
                });
                cache.insert(1, generation, &index, &block);
                pinned.push(block);
            }
        }
        assert_eq!(
            cache
                .entries
                .iter()
                .filter(|entry| entry.retained.is_some())
                .map(|entry| entry.rows.len())
                .sum::<usize>(),
            8192
        );
        assert!(
            Arc::ptr_eq(&cache.get(0, &index, &(0..256)).unwrap(), &pinned[0]),
            "caller-pinned eviction stays shared"
        );
        for common in 2..10 {
            let mut commit = commit.clone();
            commit.summary = "x".repeat(1024 * 1024).into();
            let block = Arc::new(HistoryRange {
                snapshot: index.snapshot.clone(),
                start: 0,
                commits: vec![commit; 8],
            });
            cache.insert(common, common, &index, &block);
            pinned.push(block);
        }
        assert!(
            cache
                .entries
                .iter()
                .filter(|entry| entry.retained.is_some())
                .map(|entry| entry.bytes)
                .sum::<usize>()
                <= BYTE_LIMIT
        );
        drop(index);
        cache.prune();
        assert!(cache.entries.iter().all(|entry| entry.retained.is_none()));
        drop(pinned);
        cache.prune();
        assert!(cache.entries.is_empty());
    }
}
