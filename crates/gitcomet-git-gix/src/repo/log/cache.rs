use super::authors::HistoryAuthorsCache;
use super::decode::AuthorFilter;
use super::topology::TopologyWalk;
use super::walk::{CancellableLogWalkFind, FirstParentChains, LogWalkCancellation};
use crate::repo::ShallowSnapshot;
use crate::repo::config::{ConfigInputs, ConfigStamp};
use crate::repo::shared::SharedStore;
use gitcomet_core::domain::{Commit, CommitId, HistoryMode, LogPage};
use gitcomet_core::services::Result;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(in crate::repo) struct HistoryCache {
    pub(super) pages: Mutex<Vec<LogPageCacheEntry>>,
    pub(super) authors: Mutex<Option<HistoryAuthorsCache>>,
    pub(super) range_reader: Mutex<Option<RangeReader>>,
    handle: Mutex<Option<HistoryHandle>>,
    pub(super) tips: Mutex<Option<AllBranchesTipsCacheEntry>>,
    pub(super) file_follow: Mutex<Vec<LogFileFollowCacheEntry>>,
    pub(super) followed_names: Mutex<Vec<(LogFileFollowCacheKey, Arc<FollowedNames>)>>,
    pub(super) walks: Mutex<LogPagedWalkCache>,
}

impl HistoryCache {
    pub(in crate::repo) fn invalidate_store(&self) {
        *self.range_reader.lock().expect("range reader") = None;
        *self.handle.lock().expect("history handle") = None;
        self.walks
            .lock()
            .expect("log paged walk cache")
            .entries
            .clear();
        self.pages.lock().expect("log page cache").clear();
        *self.tips.lock().expect("all branches tips") = None;
        *self.authors.lock().expect("history authors") = None;
    }

    /// The handle of the last fresh open while a reopen would read the same
    /// config and attach `installed` again. Packed refs are reloaded here on
    /// any change of their stamp: gix alone only notices a newer mtime.
    pub(in crate::repo) fn current_handle(
        &self,
        installed: &Arc<SharedStore>,
        private_scope: u64,
    ) -> Option<gix::ThreadSafeRepository> {
        let mut slot = self.handle.lock().expect("history handle");
        let handle = slot.as_mut()?;
        if !Arc::ptr_eq(&handle.shared, installed) || !handle.config.is_current(&handle.repo) {
            return None;
        }
        let repo = handle.repo.to_thread_local();
        let packed_refs = ConfigStamp::read(&repo.refs.packed_refs_path()).ok();
        if packed_refs != handle.packed_refs {
            if packed_refs.is_some() {
                repo.refs.force_refresh_packed_buffer().ok()?;
            }
            handle.packed_refs = packed_refs;
        }
        installed
            .is_current(&repo, private_scope)
            .then(|| handle.repo.clone())
    }

    pub(in crate::repo) fn keep_handle(
        &self,
        repo: &gix::ThreadSafeRepository,
        shared: &Arc<SharedStore>,
    ) {
        let local = repo.to_thread_local();
        *self.handle.lock().expect("history handle") = Some(HistoryHandle {
            config: ConfigInputs::read(&local),
            packed_refs: ConfigStamp::read(&local.refs.packed_refs_path()).ok(),
            repo: repo.clone(),
            shared: Arc::clone(shared),
        });
    }

    /// The object store for indexed range reads, re-opened every
    /// [`range_reader_blocks`] blocks. Range reads touch commit objects
    /// across the whole pack set, and every page of a mapped pack they touch
    /// stays resident until the mapping is dropped; scrolling a large history
    /// this way grew resident memory by gigabytes. A fresh open releases the
    /// mappings, so the store's footprint stays bounded by the blocks read
    /// since.
    pub(in crate::repo) fn range_reader_repo(
        &self,
        reopen: impl FnOnce() -> Result<gix::Repository>,
    ) -> Result<gix::Repository> {
        let mut slot = self.range_reader.lock().expect("range reader");
        if slot
            .as_ref()
            .is_none_or(|reader| reader.blocks >= reader.blocks_max)
        {
            gitcomet_core::history_perf::record(
                gitcomet_core::history_perf::Work::RangeStoreReopen,
            );
            let repo = reopen()?.into_sync();
            *slot = Some(RangeReader {
                blocks_max: range_reader_blocks(pack_index_bytes(&repo)),
                repo,
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

/// What `git log --follow --name-status` printed for a file: each commit's
/// status line, from which the file's name in that commit is read.
#[derive(Debug, Default)]
pub(super) struct FollowedNames {
    /// Distinct status lines; a file keeps a name for long runs of commits.
    lines: Vec<Box<str>>,
    by_commit: rustc_hash::FxHashMap<gix::ObjectId, u32>,
}

impl FollowedNames {
    /// From records of `<0x1e><hash>` followed by name-status lines. The
    /// pathspec limits those to the followed file, so the first one is its.
    pub(super) fn parse(output: &str) -> Self {
        let mut names = Self::default();
        let mut line_ids = rustc_hash::FxHashMap::<&str, u32>::default();
        for record in output.split('\u{1e}') {
            let mut lines = record.lines().filter(|line| !line.is_empty());
            let (Some(hash), Some(status)) = (lines.next(), lines.next()) else {
                continue;
            };
            let Ok(id) = gix::ObjectId::from_hex(hash.as_bytes()) else {
                continue;
            };
            let line = *line_ids.entry(status).or_insert_with(|| {
                names.lines.push(status.into());
                (names.lines.len() - 1) as u32
            });
            names.by_commit.entry(id).or_insert(line);
        }
        names
    }

    pub(super) fn status_line(&self, commit: &str) -> Option<&str> {
        let id = gix::ObjectId::from_hex(commit.as_bytes()).ok()?;
        Some(&self.lines[*self.by_commit.get(&id)? as usize])
    }

    pub(super) fn commits(&self) -> usize {
        self.by_commit.len()
    }
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
    /// `DateOrder` along first parents, for a first-parent walk from several
    /// tips. gix's topo walk follows only first parents in that mode but still
    /// reports every parent; `next` trims them, so the page and its graph see
    /// the history the walk actually took.
    FirstParentDateOrder(gix::traverse::commit::Topo<CancellableLogWalkFind, LogPagedWalkFilter>),
    /// The same order where the topo walk cannot be built: a shallow boundary.
    FirstParentChains(FirstParentChains),
}

impl LogPagedWalk {
    /// Walks that hold state proportional to the history they cover.
    pub(super) fn is_date_order(&self) -> bool {
        matches!(
            self,
            Self::DateOrder(_)
                | Self::CachedDateOrder(_)
                | Self::FirstParentDateOrder(_)
                | Self::FirstParentChains(_)
        )
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
            Self::FirstParentDateOrder(walk) => walk.next().map(|info| {
                info.map(|mut info| {
                    info.parent_ids.truncate(1);
                    info
                })
                .map_err(Into::into)
            }),
            Self::FirstParentChains(walk) => walk.next().map(Ok),
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
pub(super) const FOLLOWED_NAMES_CACHE_LIMIT: usize = 8;
/// Commits across all cached name walks, about 40 bytes each.
pub(super) const FOLLOWED_NAMES_CACHE_COMMIT_LIMIT: usize = 1_000_000;
pub(super) const LOG_PAGED_WALK_CACHE_LIMIT: usize = 32;
/// Date-order walks retain in-degree state for the reachable history.
pub(super) const LOG_PAGED_TOPO_WALK_CACHE_LIMIT: usize = 4;

/// See [`HistoryCache::current_handle`].
struct HistoryHandle {
    repo: gix::ThreadSafeRepository,
    shared: Arc<SharedStore>,
    config: ConfigInputs,
    packed_refs: Option<ConfigStamp>,
}

/// See [`HistoryCache::range_reader_repo`].
pub(super) struct RangeReader {
    repo: gix::ThreadSafeRepository,
    blocks: usize,
    blocks_max: usize,
}

/// Blocks of 256 commits read through one range-reader store before it is
/// re-opened. On chromium a block touches roughly 0.2 MiB of pack pages.
const RANGE_READER_BLOCKS: usize = 64;

/// Pack index bytes that let a store read one block more. A new store reads
/// its pack indexes from end to end before its first lookup, and its lookups
/// map them again: with chromium's 1 GB of them, re-opening every 64 blocks
/// took more time than reading the blocks.
const RANGE_READER_INDEX_BYTES_PER_BLOCK: u64 = 1 << 20;

fn range_reader_blocks(pack_index_bytes: u64) -> usize {
    let more = pack_index_bytes / RANGE_READER_INDEX_BYTES_PER_BLOCK;
    RANGE_READER_BLOCKS.saturating_add(usize::try_from(more).unwrap_or(usize::MAX))
}

/// The size of the pack indexes `repo` reads objects through, its
/// alternates' included. What cannot be read counts as nothing.
fn pack_index_bytes(repo: &gix::ThreadSafeRepository) -> u64 {
    let objects = repo.objects.path().to_owned();
    let alternates = gix::odb::alternate::resolve(
        objects.clone(),
        &std::env::current_dir().unwrap_or_default(),
    )
    .unwrap_or_default();
    std::iter::once(objects)
        .chain(alternates)
        .filter_map(|objects| std::fs::read_dir(objects.join("pack")).ok())
        .flatten()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let is_index = entry.path().extension().is_some_and(|ext| ext == "idx");
            is_index
                .then(|| entry.metadata().ok())?
                .map(|file| file.len())
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{git_success, init_test_repo, open_repo, write_file};

    #[test]
    fn a_store_reads_more_blocks_the_larger_its_pack_indexes() {
        assert_eq!(range_reader_blocks(0), 64);
        assert_eq!(range_reader_blocks((1 << 20) - 1), 64);
        assert_eq!(range_reader_blocks(1 << 30), 64 + 1024);
    }

    #[test]
    fn a_range_reader_lasts_longer_beside_larger_pack_indexes() {
        let dir = tempfile::tempdir().unwrap();
        init_test_repo(dir.path());
        let repo = open_repo(dir.path());
        let cache = HistoryCache::default();
        let reopens = std::cell::Cell::new(0);
        let read = |blocks: usize| {
            for _ in 0..blocks {
                let reopen = || {
                    reopens.set(reopens.get() + 1);
                    Ok(repo.repo())
                };
                cache.range_reader_repo(reopen).unwrap();
            }
            reopens.get()
        };
        assert_eq!(read(64), 1);

        // Two megabytes of pack index, as far as a directory listing shows.
        let index = dir.path().join(".git/objects/pack/pack-large.idx");
        let file = std::fs::File::create(index).unwrap();
        file.set_len(2 << 20).unwrap();
        assert_eq!(read(66), 2);
        assert_eq!(read(1), 3);
    }

    #[test]
    fn pack_index_bytes_include_those_of_alternates() {
        let packed = |root: &std::path::Path, files: usize| {
            init_test_repo(root);
            for file in 0..files {
                write_file(root, &format!("{file}.txt"), &format!("{file}\n"));
            }
            git_success(root, &["add", "-A"]);
            git_success(root, &["commit", "-qm", "files"]);
            git_success(root, &["repack", "-a", "-d", "-q"]);
            let index = std::fs::read_dir(root.join(".git/objects/pack"))
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .find(|path| path.extension().is_some_and(|ext| ext == "idx"))
                .unwrap();
            std::fs::metadata(index).unwrap().len()
        };
        let (here, there) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let own = packed(here.path(), 1);
        let borrowed = packed(there.path(), 40);
        assert_ne!(own, borrowed);
        let bytes = || pack_index_bytes(&open_repo(here.path()).repo().into_sync());
        assert_eq!(bytes(), own);

        write_file(
            here.path(),
            ".git/objects/info/alternates",
            &format!("{}\n", there.path().join(".git/objects").display()),
        );
        assert_eq!(bytes(), own + borrowed);
    }
}
