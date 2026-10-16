use super::authors::HistoryAuthorsCache;
use super::decode::AuthorFilter;
use super::topology::TopologyWalk;
use super::walk::{CancellableLogWalkFind, LogWalkCancellation};
use crate::repo::ShallowSnapshot;
use gitcomet_core::domain::{Commit, CommitId, HistoryMode, LogPage};
use gitcomet_core::services::Result;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(in crate::repo) struct HistoryCache {
    pub(super) pages: Mutex<Vec<LogPageCacheEntry>>,
    pub(super) authors: Mutex<Option<HistoryAuthorsCache>>,
    pub(super) range_reader: Mutex<Option<RangeReader>>,
    pub(super) tips: Mutex<Option<AllBranchesTipsCacheEntry>>,
    pub(super) file_follow: Mutex<Vec<LogFileFollowCacheEntry>>,
    pub(super) walks: Mutex<LogPagedWalkCache>,
}

impl HistoryCache {
    pub(in crate::repo) fn invalidate_store(&self) {
        *self.range_reader.lock().expect("range reader") = None;
        self.walks
            .lock()
            .expect("log paged walk cache")
            .entries
            .clear();
        self.pages.lock().expect("log page cache").clear();
        *self.tips.lock().expect("all branches tips") = None;
        *self.authors.lock().expect("history authors") = None;
    }

    /// The object store for indexed range reads, re-opened every
    /// [`RANGE_READER_REOPEN_BLOCKS`] blocks. Range reads touch commit objects
    /// across the whole pack set, and every page of a mapped pack they touch
    /// stays resident until the mapping is dropped; scrolling a large history
    /// this way grew resident memory by gigabytes. A fresh open costs a config
    /// parse and releases the mappings, so the store's footprint stays bounded
    /// by the blocks read since.
    pub(in crate::repo) fn range_reader_repo(
        &self,
        reopen: impl FnOnce() -> Result<gix::Repository>,
    ) -> Result<gix::Repository> {
        let mut slot = self.range_reader.lock().expect("range reader");
        if slot
            .as_ref()
            .is_none_or(|reader| reader.blocks >= RANGE_READER_REOPEN_BLOCKS)
        {
            gitcomet_core::history_perf::record(
                gitcomet_core::history_perf::Work::RangeStoreReopen,
            );
            *slot = Some(RangeReader {
                repo: reopen()?.into_sync(),
                blocks: 0,
            });
        }
        let reader = slot.as_mut().expect("range reader is open");
        reader.blocks += 1;
        Ok(reader.repo.to_thread_local())
    }
}

/// What a cached log page was walked from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum LogPageSeed {
    Head(Option<gix::ObjectId>),
    Tips(Arc<[gix::ObjectId]>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LogPageCacheKey {
    pub(super) generation: u64,
    pub(super) mode: HistoryMode,
    pub(super) seed: LogPageSeed,
    /// Invalidates cached pages when a shallow repository is deepened or its
    /// boundary is otherwise replaced without moving a ref.
    pub(super) shallow: ShallowSnapshot,
    pub(super) limit: usize,
    pub(super) last_seen: Option<CommitId>,
    pub(super) resume_from: Option<CommitId>,
    /// Author filter, or `None` for the unfiltered walk.
    pub(super) author: Option<AuthorFilter>,
}

#[derive(Clone, Debug)]
pub(super) struct LogPageCacheEntry {
    pub(super) key: LogPageCacheKey,
    pub(super) page: Arc<LogPage>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LogFileFollowCacheKey {
    pub(super) head_oid: Option<gix::ObjectId>,
    pub(super) path: PathBuf,
}

#[derive(Clone, Debug)]
pub(super) struct LogFileFollowCacheEntry {
    pub(super) key: LogFileFollowCacheKey,
    pub(super) commits: Arc<Vec<Commit>>,
}

/// The paged walk's commit filter. Boxed rather than a plain `fn` because a
/// shallow repository needs one that carries state — the grafted parents still
/// to be skipped — and the walk it belongs to is parked in [`LogPagedWalkCache`],
/// so it can borrow nothing.
pub(super) type LogPagedWalkFilter = Box<dyn FnMut(&gix::oid) -> bool + Send>;

pub(super) enum LogPagedWalk {
    CommitTime(gix::traverse::commit::Simple<gix::OdbHandleArc, LogPagedWalkFilter>),
    DateOrder(gix::traverse::commit::Topo<CancellableLogWalkFind, LogPagedWalkFilter>),
    CachedDateOrder(TopologyWalk),
}

impl LogPagedWalk {
    pub(super) fn is_date_order(&self) -> bool {
        matches!(self, Self::DateOrder(_) | Self::CachedDateOrder(_))
    }
}

impl Iterator for LogPagedWalk {
    type Item = std::result::Result<
        gix::traverse::commit::Info,
        Box<dyn std::error::Error + Send + Sync + 'static>,
    >;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::CommitTime(walk) => walk.next().map(|info| info.map_err(Into::into)),
            Self::DateOrder(walk) => walk.next().map(|info| info.map_err(Into::into)),
            Self::CachedDateOrder(walk) => walk.next().map(Ok),
        }
    }
}

pub(super) struct LogPagedWalkState {
    pub(super) pending: std::collections::VecDeque<gix::traverse::commit::Info>,
    pub(super) walk: LogPagedWalk,
    pub(super) cancellation: LogWalkCancellation,
}

pub(super) struct LogPagedWalkCacheEntry {
    pub(super) token: Arc<str>,
    pub(super) mode: HistoryMode,
    /// The commits the walk was seeded from — one head for most modes, every
    /// ref for `AllBranches`. A walk started from different tips covers a
    /// different history, so a token minted for one must not resume the other.
    pub(super) tips: Arc<[gix::ObjectId]>,
    /// The exact shallow boundary the walk was built against. A cursor minted
    /// before a deepen must not resume the old, truncated walk.
    pub(super) shallow: ShallowSnapshot,
    /// Author filter the walk was started with, or `None` for the unfiltered
    /// walk. The walk's *position* depends on the filter — every non-matching
    /// commit was already consumed — so resuming one walk under a different
    /// filter would silently skip whatever the first pass rejected.
    pub(super) author: Option<AuthorFilter>,
    pub(super) state: LogPagedWalkState,
    /// The store generation the walk reads from; see [`crate::repo::GixRepo::reopen_object_store`].
    pub(super) generation: u64,
}

#[derive(Default)]
pub(super) struct LogPagedWalkCache {
    pub(super) next_id: u64,
    pub(super) entries: Vec<LogPagedWalkCacheEntry>,
}

#[derive(Clone, Debug)]
pub(super) struct AllBranchesTipsCacheEntry {
    pub(super) fingerprint: u64,
    pub(super) tips: Arc<[gix::ObjectId]>,
}
pub(super) const LOG_PAGE_CACHE_LIMIT: usize = 32;
pub(super) const LOG_PAGE_CACHE_ROW_LIMIT: usize = 10_000;
pub(super) const LOG_FILE_FOLLOW_CACHE_LIMIT: usize = 16;
pub(super) const LOG_PAGED_WALK_CACHE_LIMIT: usize = 32;
/// Date-order walks retain in-degree state for the reachable history.
pub(super) const LOG_PAGED_TOPO_WALK_CACHE_LIMIT: usize = 4;

/// See [`HistoryCache::range_reader_repo`].
pub(super) struct RangeReader {
    repo: gix::ThreadSafeRepository,
    blocks: usize,
}

/// Blocks of 256 commits read through one range-reader store before it is
/// re-opened. On chromium a block touches roughly 0.2 MiB of pack pages.
const RANGE_READER_REOPEN_BLOCKS: usize = 64;
