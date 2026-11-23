//! Optional decoded retention is bounded globally; caller-pinned blocks are
//! registered weakly and remain reusable even after their retention is evicted.
use super::GixRepo;
use gitcomet_core::history_index::{
    HISTORY_ROW_CACHE_LIMIT, HistoryIndex, HistoryIndexHandle, HistoryRange,
};
use gitcomet_core::services::{CancellationToken, Result};
use std::collections::VecDeque;
use std::ops::Range;
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::Duration;

const BYTE_LIMIT: usize = 32 * 1024 * 1024;
/// How long a waiter sleeps between checks of its own cancellation.
const WAIT: Duration = Duration::from_millis(20);

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

/// The index's address and rows. A flight's leader keeps the index alive.
type BlockKey = (usize, Range<usize>);

/// Blocks being decoded for one store: one decode per block, while other
/// blocks decode in parallel.
#[derive(Default)]
pub(super) struct DecodeFlights {
    active: Mutex<Vec<BlockKey>>,
    changed: Condvar,
}

/// The claim on one block. Dropping it wakes that block's waiters, also when
/// the decode failed or was cancelled, so one of them can take over.
struct DecodeFlight<'a> {
    flights: &'a DecodeFlights,
    key: BlockKey,
    /// Taken over from a decode that failed or stopped.
    waited: bool,
}

impl Drop for DecodeFlight<'_> {
    fn drop(&mut self) {
        let mut active = self
            .flights
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        active.retain(|key| key != &self.key);
        drop(active);
        self.flights.changed.notify_all();
    }
}

impl DecodeFlights {
    /// The decoded block, or the claim to decode it. A caller whose block is
    /// in flight waits here and stays responsive to its own cancellation.
    fn join(
        &self,
        generation: u64,
        index: &HistoryIndexHandle,
        rows: &Range<usize>,
        cancellation: &CancellationToken,
    ) -> Result<std::result::Result<Arc<HistoryRange>, DecodeFlight<'_>>> {
        let key = (Arc::as_ptr(index) as usize, rows.clone());
        let mut active = self.active.lock().expect("range decode flights");
        let mut waited = false;
        loop {
            cancellation.check_cancelled()?;
            if !active.contains(&key) {
                // A leader publishes before it releases its claim, so a miss
                // here means nobody holds this block.
                if let Some(block) = CACHE
                    .lock()
                    .expect("shared ranges")
                    .get(generation, index, rows)
                {
                    return Ok(Ok(block));
                }
                active.push(key.clone());
                return Ok(Err(DecodeFlight {
                    flights: self,
                    key,
                    waited,
                }));
            }
            waited = true;
            active = self
                .changed
                .wait_timeout(active, WAIT)
                .expect("range decode flights")
                .0;
        }
    }
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
        self.read_shared_history_range_with(index, rows.clone(), cancellation, || {
            self.decode_history_range(index, rows, cancellation)
        })
    }

    fn read_shared_history_range_with(
        &self,
        index: &HistoryIndexHandle,
        rows: Range<usize>,
        cancellation: &CancellationToken,
        decode: impl FnOnce() -> Result<HistoryRange>,
    ) -> Result<Arc<HistoryRange>> {
        cancellation.check_cancelled()?;
        let (_, shared) = self.current_history_store()?;
        self.validate_history_store(index, &shared)?;
        // Decode a block once for concurrent callers, without holding the
        // global cache lock, or any other, through I/O.
        let flight = match shared.decode.join(shared.id, index, &rows, cancellation)? {
            Ok(block) => return Ok(block),
            Err(flight) => flight,
        };
        if flight.waited {
            // The store may have changed during the wait.
            let (_, current) = self.current_history_store()?;
            self.validate_history_store(index, &current)?;
        }
        let block = Arc::new(decode()?);
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
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    fn two_commits() -> (tempfile::TempDir, GixRepo, HistoryIndexHandle) {
        let dir = tempfile::tempdir().unwrap();
        crate::test_support::init_test_repo(dir.path());
        for message in ["one", "two"] {
            crate::test_support::git_success(
                dir.path(),
                &["commit", "--allow-empty", "-qm", message],
            );
        }
        let repo = crate::test_support::open_repo(dir.path());
        let index = repo
            .build_history_index_impl(
                &HistoryMode::FullReachable.into(),
                None,
                &Default::default(),
                &CancellationToken::new(),
                &mut |_| {},
            )
            .unwrap()
            .unwrap();
        (dir, repo, index)
    }

    #[test]
    fn different_blocks_decode_concurrently() {
        let (_dir, repo, index) = two_commits();
        let cancel = CancellationToken::new();
        let (active, peak) = (AtomicUsize::new(0), AtomicUsize::new(0));
        std::thread::scope(|scope| {
            for rows in [0..1, 1..2] {
                let (repo, index, cancel, active, peak) = (&repo, &index, &cancel, &active, &peak);
                scope.spawn(move || {
                    repo.read_shared_history_range_with(index, rows.clone(), cancel, || {
                        peak.fetch_max(active.fetch_add(1, SeqCst) + 1, SeqCst);
                        // Stay in the decode until the other block joins it.
                        let deadline = Instant::now() + Duration::from_secs(2);
                        while peak.load(SeqCst) < 2 && Instant::now() < deadline {
                            std::thread::sleep(Duration::from_millis(1));
                        }
                        active.fetch_sub(1, SeqCst);
                        repo.read_history_range_impl(index, rows, cancel)
                    })
                    .unwrap();
                });
            }
        });
        assert_eq!(
            peak.load(SeqCst),
            2,
            "one block's decode must not wait for another's"
        );
    }

    #[test]
    fn a_same_block_waiter_receives_the_block_decoded_once() {
        let (_dir, repo, index) = two_commits();
        let cancel = CancellationToken::new();
        let decodes = AtomicUsize::new(0);
        let (started_tx, started) = mpsc::channel();
        let (release, gate) = mpsc::channel::<()>();
        let (repo, index, cancel, decodes) = (&repo, &index, &cancel, &decodes);
        std::thread::scope(move |scope| {
            let leader = scope.spawn(move || {
                repo.read_shared_history_range_with(index, 0..2, cancel, || {
                    decodes.fetch_add(1, SeqCst);
                    started_tx.send(()).unwrap();
                    gate.recv().unwrap();
                    repo.read_history_range_impl(index, 0..2, cancel)
                })
            });
            started.recv().unwrap();
            let waiter = scope.spawn(move || {
                repo.read_shared_history_range_with(index, 0..2, cancel, || {
                    decodes.fetch_add(1, SeqCst);
                    repo.read_history_range_impl(index, 0..2, cancel)
                })
            });
            // Long enough for a waiter that wrongly decodes to have started.
            std::thread::sleep(Duration::from_millis(50));
            release.send(()).unwrap();
            let leader = leader.join().unwrap().unwrap();
            assert!(Arc::ptr_eq(&leader, &waiter.join().unwrap().unwrap()));
        });
        assert_eq!(decodes.load(SeqCst), 1);
    }

    /// A waiter that takes over a failed decode checks the store again: a
    /// replace ref made during the wait changes what the index's rows mean.
    #[test]
    fn a_waiter_taking_over_a_failed_decode_sees_a_changed_store() {
        let (dir, repo, index) = two_commits();
        let cancel = CancellationToken::new();
        let decodes = AtomicUsize::new(0);
        let (started_tx, started) = mpsc::channel();
        let (release, gate) = mpsc::channel::<()>();
        let (repo, index, cancel, decodes) = (&repo, &index, &cancel, &decodes);
        std::thread::scope(move |scope| {
            let leader = scope.spawn(move || {
                repo.read_shared_history_range_with(index, 0..2, cancel, || {
                    started_tx.send(()).unwrap();
                    gate.recv().unwrap();
                    Err(gitcomet_core::error::Error::new(
                        gitcomet_core::error::ErrorKind::Backend("decode failed".into()),
                    ))
                })
            });
            started.recv().unwrap();
            let waiter = scope.spawn(move || {
                repo.read_shared_history_range_with(index, 0..2, cancel, || {
                    decodes.fetch_add(1, SeqCst);
                    repo.read_history_range_impl(index, 0..2, cancel)
                })
            });
            // Let the waiter reach its wait.
            std::thread::sleep(Duration::from_millis(50));
            crate::test_support::git_success(dir.path(), &["replace", "--graft", "HEAD"]);
            release.send(()).unwrap();
            assert!(leader.join().unwrap().is_err());
            let error = waiter.join().unwrap().unwrap_err();
            assert!(
                error.to_string().contains("interpretation changed"),
                "{error}"
            );
        });
        assert_eq!(decodes.load(SeqCst), 0);
    }

    #[test]
    fn a_cancelled_waiter_leaves_while_the_decode_completes() {
        let (_dir, repo, index) = two_commits();
        let (leader_cancel, waiter_cancel) = (CancellationToken::new(), CancellationToken::new());
        let (started_tx, started) = mpsc::channel();
        let (release, gate) = mpsc::channel::<()>();
        let (done_tx, done) = mpsc::channel();
        let (repo, index) = (&repo, &index);
        let (leader_cancel, waiter_cancel) = (&leader_cancel, &waiter_cancel);
        std::thread::scope(move |scope| {
            let leader = scope.spawn(move || {
                repo.read_shared_history_range_with(index, 0..2, leader_cancel, || {
                    started_tx.send(()).unwrap();
                    gate.recv().unwrap();
                    repo.read_history_range_impl(index, 0..2, leader_cancel)
                })
            });
            started.recv().unwrap();
            scope.spawn(move || {
                let result = repo.read_shared_history_range(index, 0..2, waiter_cancel);
                done_tx.send(result).unwrap();
            });
            // Let the waiter reach its wait before it is cancelled.
            std::thread::sleep(Duration::from_millis(50));
            waiter_cancel.cancel();
            let result = done
                .recv_timeout(Duration::from_secs(2))
                .expect("a cancelled waiter must not wait for the decode");
            assert!(matches!(
                result.unwrap_err().kind(),
                gitcomet_core::error::ErrorKind::Cancelled
            ));
            release.send(()).unwrap();
            assert_eq!(leader.join().unwrap().unwrap().commits.len(), 2);
        });
    }

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
