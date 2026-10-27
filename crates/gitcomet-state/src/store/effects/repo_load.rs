use crate::model::{AppState, RepoLifetime, WorktreeDirtyScope};
use crate::msg::{Msg, WorktreeListRead};
use crate::store::repository_runtime::RepositoryRuntimes;
use gitcomet_core::domain::{
    DiffPreviewTextSide, DiffTarget, LogCursor, LogScope, RepoStatus, Worktree,
    WorktreeDirtySummary, count_file_statuses,
};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::{CancellationToken, GitBackend, GitRepository};
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use super::super::{
    RepoId, executor::TaskExecutor, repo_load_trace, worker_channel::StoreWorkerSender,
};
use super::util::{
    missing_repo_error, send_or_log, spawn_detached_with_repo_or_else, spawn_with_repo,
    spawn_with_repo_or_else,
};
use super::worktree_list::WorktreeListPublisher;

pub(super) struct SelectedDiffLoadOptions {
    /// Read the file's `.gitattributes` and text config.
    pub(super) load_text_attributes: bool,
    pub(super) load_patch_diff: bool,
    pub(super) load_file_text: bool,
    pub(super) preview_text_side: Option<DiffPreviewTextSide>,
    pub(super) load_submodule_summary: bool,
    pub(super) load_file_image: bool,
}

#[derive(Clone)]
pub(super) struct SelectedDiffLoadGuard {
    thread_state: Arc<RwLock<Arc<AppState>>>,
    repo_id: RepoId,
    target: DiffTarget,
    target_rev: u64,
}

impl SelectedDiffLoadGuard {
    pub(super) fn new(
        thread_state: Arc<RwLock<Arc<AppState>>>,
        repo_id: RepoId,
        target: DiffTarget,
        target_rev: u64,
    ) -> Self {
        Self {
            thread_state,
            repo_id,
            target,
            target_rev,
        }
    }

    fn is_current(&self) -> bool {
        let state = self.thread_state.read().unwrap_or_else(|e| e.into_inner());
        state
            .repos
            .iter()
            .find(|repo| repo.id == self.repo_id)
            .is_some_and(|repo| {
                repo.diff_state.diff_target_rev == self.target_rev
                    && repo.diff_state.diff_target.as_ref() == Some(&self.target)
            })
    }
}

/// Traces the same queue/start/finish/skip events as `spawn_with_repo`, plus
/// the queued request a newer one replaces.
fn spawn_with_selected_diff_guard(
    executor: &TaskExecutor,
    slot: &super::super::executor::LatestTaskSlot,
    task_name: &'static str,
    repos: &RepositoryRuntimes,
    repo_id: RepoId,
    msg_tx: StoreWorkerSender,
    guard: SelectedDiffLoadGuard,
    task: impl FnOnce(Arc<dyn GitRepository>, StoreWorkerSender, SelectedDiffLoadGuard) + Send + 'static,
) -> bool {
    let Some(repo) = repos.repository(&repo_id).cloned() else {
        repo_load_trace::trace!("repo_task_missing_handle repo_id={repo_id:?} task={task_name}");
        return false;
    };
    // Trace before enqueueing: the worker may start the task immediately.
    repo_load_trace::trace!("queue_repo_task repo_id={repo_id:?} task={task_name}");
    let replaced = executor.spawn_latest(slot, move || {
        if msg_tx.is_cancelled() {
            repo_load_trace::trace!(
                "skip_repo_task_cancelled_before_start repo_id={repo_id:?} task={task_name}"
            );
            return;
        }
        if !guard.is_current() {
            repo_load_trace::trace!(
                "skip_repo_task_stale_selection repo_id={repo_id:?} task={task_name}"
            );
            return;
        }
        repo_load_trace::trace!("start_repo_task repo_id={repo_id:?} task={task_name}");
        task(repo, msg_tx, guard);
        repo_load_trace::trace!("finish_repo_task repo_id={repo_id:?} task={task_name}");
    });
    if replaced {
        repo_load_trace::trace!("replace_queued_repo_task repo_id={repo_id:?} task={task_name}");
    }
    true
}

#[cfg(test)]
mod selected_diff_guard_tests;

#[allow(clippy::too_many_arguments)]
pub(super) fn schedule_load_log(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    seq: crate::model::LogLoadSeq,
    scope: LogScope,
    author: Option<String>,
    cursor: Option<LogCursor>,
    request: gitcomet_core::services::HistoryReadRequest,
    cancellation: CancellationToken,
) {
    let cursor_on_missing = cursor.clone();
    spawn_detached_with_repo_or_else(
        executor,
        "load-log",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            let mut on_chunk = |chunk: gitcomet_core::services::LogChunk| {
                send_or_log(
                    &msg_tx,
                    Msg::Internal(crate::msg::InternalMsg::LogChunkLoaded {
                        repo_id,
                        seq,
                        commits: chunk.commits,
                        scanned: chunk.scanned,
                    }),
                );
            };
            let result = repo.read_history(
                scope,
                author.as_deref(),
                &request,
                &cancellation,
                &mut on_chunk,
            );
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::LogLoaded {
                    repo_id,
                    seq,
                    scope,
                    cursor,
                    result,
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::LogLoaded {
                    repo_id,
                    seq,
                    scope,
                    cursor: cursor_on_missing,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_stashes(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    limit: usize,
    cancellation: CancellationToken,
) {
    spawn_detached_with_repo_or_else(
        executor,
        "load-stashes",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            let mut entries = repo.stash_list(&cancellation);
            if let Ok(v) = &mut entries {
                v.truncate(limit);
            }
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::StashesLoaded {
                    repo_id,
                    result: entries,
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::StashesLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_reflog(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    limit: usize,
) {
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::ReflogLoaded {
                    repo_id,
                    result: repo.reflog_head(limit),
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::ReflogLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

/// Blame for `path` at `source`: History's and diff sessions' shared reader.
pub(super) fn load_blame(
    repo: &dyn GitRepository,
    path: &Path,
    source: &gitcomet_core::domain::BlameSource,
) -> gitcomet_core::services::Result<Vec<gitcomet_core::services::BlameLine>> {
    use gitcomet_core::domain::BlameSource;
    match source {
        BlameSource::Revision(rev) => repo.blame_file(path, rev.as_deref()),
        BlameSource::WorkingTree(area) => repo.blame_worktree_file(path, *area),
    }
}

pub(super) fn schedule_load_blame(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    path: PathBuf,
    source: gitcomet_core::domain::BlameSource,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = load_blame(repo.as_ref(), &path, &source);
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::BlameLoaded {
                repo_id,
                path: path.clone(),
                source,
                result,
            }),
        );
    });
}

pub(super) fn schedule_load_worktrees(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    cancellation: CancellationToken,
    publisher: WorktreeListPublisher,
) {
    spawn_detached_with_repo_or_else(
        executor,
        "load-worktrees",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            let read = WorktreeListRead::explicit();
            let result = repo.list_worktrees(&cancellation).map(Arc::new);
            publisher.publish(&msg_tx, repo_id, read, result);
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::WorktreesLoaded {
                    repo_id,
                    read: WorktreeListRead::explicit(),
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

/// Scans all other linked worktrees, or just the requested checkouts.
///
/// The worktree list is re-read inside the task rather than passed in from
/// state: this way the scan can never disagree with the paths it walks, and it
/// has no ordering dependency on `LoadWorktrees` having landed first. Full scans
/// publish changed listings before status work, keeping the sidebar current
/// even when checkouts are inaccessible. Targeted scans avoid another state
/// publication on every file edit; git-state changes already request full scans.
///
/// A worktree that cannot be opened or stat'd is skipped rather than failing the
/// whole scan — removed directories and unmounted volumes are routine.
#[allow(clippy::too_many_arguments)]
pub(super) fn schedule_load_worktree_dirty(
    executor: &TaskExecutor,
    backend: Arc<dyn GitBackend>,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    cache_lease: super::WorktreeCacheLease,
    own_workdir: PathBuf,
    scope: WorktreeDirtyScope,
    files_for: Option<PathBuf>,
    cancellation: CancellationToken,
    publisher: WorktreeListPublisher,
) {
    let missing_scope = scope.clone();
    spawn_detached_with_repo_or_else(
        executor,
        "load-worktree-dirty",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            let lifetime = cache_lease.lifetime();
            let own_workdir = canonicalize_or_original(own_workdir);
            let read = WorktreeListRead::from_dirty_scan();
            // gix can panic if a checkout is removed between classifying its
            // root as a repository and reading that now-empty directory. The
            // executor catches unwinds, but only this task can finish the load
            // and let the reducer replay a refresh queued during the scan.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                repo.list_worktrees(&cancellation)
                    .inspect_err(|_| {
                        // The listing is what names the live worktrees, so a failed
                        // one leaves nothing to prune against -- and its causes (a
                        // concurrent `git worktree prune`, a permissions blip, an
                        // unmounted volume) are exactly when stale handles pile up.
                        // Drop this repo's cache outright: it is only a cache, and
                        // the next scan reopens whatever it still needs. A cancelled
                        // listing is not evidence of anything, so it keeps its
                        // handles.
                        if !cancellation.is_cancelled() {
                            retain_worktree_scan_handles(worktree_scan_handles(), lifetime, &[]);
                        }
                    })
                    .and_then(|worktrees| {
                        let worktrees = Arc::new(worktrees);
                        if matches!(scope, WorktreeDirtyScope::All) {
                            // Background failures must not change the explicit list's
                            // state or produce a diagnostic on every focus/flush.
                            publisher.publish(&msg_tx, repo_id, read, Ok(worktrees.clone()));
                        }
                        let mut summaries = Vec::new();
                        let mut scanned = Vec::with_capacity(worktrees.len());
                        for worktree in worktrees.iter() {
                            // A scan that stops here has seen only a prefix of the
                            // list, and the reducer commits an `Ok` as the complete
                            // set of dirty worktrees: it would blank the rows for
                            // everything unscanned and -- through
                            // `selected_worktree_is_gone` -- drop the selection and
                            // close the diff the user is reading. Cancellation is
                            // the absence of an answer, so it surfaces as one.
                            if cancellation.is_cancelled() {
                                return Err(Error::new(ErrorKind::Cancelled));
                            }
                            if worktree.bare || is_own_worktree(&worktree.path, &own_workdir) {
                                continue;
                            }
                            scanned.push(worktree.path.clone());
                            if !scope.includes(&worktree.path) {
                                continue;
                            }
                            let Some(handle) = worktree_scan_handle(
                                worktree_scan_handles(),
                                &*backend,
                                &cache_lease,
                                &worktree.path,
                            ) else {
                                continue;
                            };
                            let status = match handle.status(&cancellation) {
                                Ok(status) => status,
                                // Cancellation surfaces as an `Err` here like any
                                // other failure, and it says nothing about the
                                // handle. `CancelRepoLoads` fires on every tab
                                // switch, reload and completed action, so treating
                                // it as a broken handle would discard healthy ones
                                // routinely and pay full discovery to reopen them.
                                Err(_) if cancellation.is_cancelled() => {
                                    return Err(Error::new(ErrorKind::Cancelled));
                                }
                                Err(_) => {
                                    // A handle that cannot report status is not
                                    // worth keeping: the worktree may have been
                                    // removed or replaced underneath it.
                                    forget_worktree_scan_handle(
                                        worktree_scan_handles(),
                                        &worktree.path,
                                    );
                                    continue;
                                }
                            };
                            if status.staged.is_empty() && status.unstaged.is_empty() {
                                continue;
                            }
                            // Only the selected worktree's files are carried back;
                            // see `WorktreeDirtySummary`.
                            let keep_files = files_for.as_deref() == Some(worktree.path.as_path());
                            // Like the file lists, only the selected worktree pays.
                            let line_stats = if keep_files {
                                match handle
                                    .uncommitted_line_stats_for_status(&status, &cancellation)
                                {
                                    Ok(stats) => stats,
                                    // Like the status read above: a cancelled scan
                                    // stops the walk rather than reporting a
                                    // worktree whose counts silently went missing.
                                    Err(_) if cancellation.is_cancelled() => {
                                        return Err(Error::new(ErrorKind::Cancelled));
                                    }
                                    // Counts are decoration; a worktree that cannot
                                    // produce them still belongs in the list.
                                    Err(_) => Default::default(),
                                }
                            } else {
                                Default::default()
                            };
                            let summary = worktree_dirty_summary(
                                worktree.clone(),
                                status,
                                keep_files,
                                line_stats,
                            );
                            if summary.is_dirty() {
                                summaries.push(summary);
                            }
                        }
                        // Reaching here means the whole list was walked, so `scanned`
                        // is complete and safe to prune against -- a cancellation
                        // part-way returned above rather than falling through with a
                        // partial set.
                        retain_worktree_scan_handles(worktree_scan_handles(), lifetime, &scanned);
                        Ok(summaries)
                    })
            }))
            .unwrap_or_else(|payload| {
                // The scan did not reach its normal pruning step. Reopen this
                // repo's checkouts next time instead of retaining stale handles.
                release_worktree_scan_handles(lifetime);
                Err(Error::new(ErrorKind::Backend(format!(
                    "Worktree scan panicked: {}",
                    gitcomet_core::panic::payload_message(payload.as_ref()),
                ))))
            });
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::WorktreeDirtyLoaded {
                    repo_id,
                    scope,
                    result,
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::WorktreeDirtyLoaded {
                    repo_id,
                    scope: missing_scope,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

/// Repository handles for the *other* worktrees, kept between scans.
///
/// The scan runs on every git-state flush, and opening a repository is discovery
/// plus config parsing — several milliseconds each, paid per worktree per scan
/// for handles that were identical the last time round. Status itself re-reads
/// the index and the worktree, so a reused handle reports fresh results; a handle
/// that fails is dropped ([`forget_worktree_scan_handle`]) and reopened next time.
///
/// Keyed by canonical checkout path so linked tabs share warm status caches.
/// Owners are tracked separately: pruning or closing one tab releases only its
/// ownership, and a handle is dropped when no tabs still need it.
static WORKTREE_SCAN_HANDLES: OnceLock<Mutex<WorktreeScanHandles>> = OnceLock::new();

/// Handles held open across scans. Each one keeps file descriptors and mapped
/// index data alive, so the total is capped rather than left to grow with every
/// worktree the session has ever looked at.
const WORKTREE_SCAN_HANDLE_LIMIT: usize = 64;

#[derive(Default)]
struct WorktreeScanHandles {
    entries: FxHashMap<PathBuf, WorktreeScanHandle>,
    /// Ticks once per lookup; the entry holding the highest tick is the hottest.
    clock: u64,
}

struct WorktreeScanHandle {
    handle: Arc<dyn GitRepository>,
    owners: FxHashSet<RepoLifetime>,
    last_used: u64,
}

impl WorktreeScanHandles {
    fn tick(&mut self) -> u64 {
        self.clock = self.clock.wrapping_add(1);
        self.clock
    }

    /// Frees a slot for a new entry belonging to `owner`.
    ///
    /// Take from the largest owner, with the requester winning ties, so a new
    /// repository can earn a fair share rather than staying pinned to whatever
    /// slots were free when it opened. Shared checkouts occupy one slot but
    /// count toward each owner's share.
    ///
    /// Within the requester's own entries the *most* recently used goes, not the
    /// coldest. A scan walks `list_worktrees` in order and every scan walks the
    /// same list, so the access pattern is a cycle -- LRU's textbook worst case.
    /// Under LRU a repo with more worktrees than the limit evicts precisely the
    /// entry its next scan reaches first, every time, and reopens all of them on
    /// every pass: the same behaviour clearing the map wholesale had. Giving up
    /// the entry the scan has just finished with instead leaves a stable prefix
    /// cached and confines the reopens to the tail.
    ///
    /// Within *another* repo's entries the coldest goes: there LRU is right,
    /// because another repo's oldest handle really is the least likely to be
    /// wanted next, and nothing about this repo's cycle says which of theirs to
    /// keep.
    fn evict_one_for(&mut self, owner: RepoLifetime) -> bool {
        let mut held: FxHashMap<RepoLifetime, usize> = FxHashMap::default();
        for entry in self.entries.values() {
            for owner in &entry.owners {
                *held.entry(*owner).or_default() += 1;
            }
        }
        let Some(largest) = held
            .into_iter()
            // The requester wins ties, so a single repo over the limit keeps the
            // cyclic behaviour below and an even split stays put. `entry_repo`
            // breaks the rest, only so the map's iteration order cannot make this
            // vary between runs.
            .max_by_key(|&(entry_repo, count)| (count, entry_repo == owner, entry_repo))
            .map(|(entry_repo, _)| entry_repo)
        else {
            return false;
        };

        let of_largest = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.owners.contains(&largest));
        let victim = if largest == owner {
            of_largest.max_by_key(|(_, entry)| entry.last_used)
        } else {
            of_largest.min_by_key(|(_, entry)| entry.last_used)
        }
        .map(|(key, _)| key.clone());

        match victim {
            Some(victim) => {
                self.entries.remove(&victim);
                true
            }
            None => false,
        }
    }
}

fn worktree_scan_handles() -> &'static Mutex<WorktreeScanHandles> {
    WORKTREE_SCAN_HANDLES.get_or_init(|| Mutex::new(WorktreeScanHandles::default()))
}

fn lock_worktree_scan_handles(
    handles: &Mutex<WorktreeScanHandles>,
) -> std::sync::MutexGuard<'_, WorktreeScanHandles> {
    handles
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The map is threaded in rather than reached for through the static so tests can
/// drive an instance of their own. Sharing the process-wide one made the handle
/// tests evict each other's entries whenever cargo ran them in parallel.
fn worktree_scan_handle(
    handles: &Mutex<WorktreeScanHandles>,
    backend: &dyn GitBackend,
    lease: &super::WorktreeCacheLease,
    path: &Path,
) -> Option<Arc<dyn GitRepository>> {
    let key = canonicalize_or_original(path.to_path_buf());
    let owner = lease.lifetime();
    {
        let mut handles = lock_worktree_scan_handles(handles);
        if !lease.is_open() {
            return None;
        }
        let now = handles.tick();
        if let Some(entry) = handles.entries.get_mut(&key) {
            entry.last_used = now;
            entry.owners.insert(owner);
            return Some(Arc::clone(&entry.handle));
        }
    }

    // Opened outside the lock: this is the several-millisecond discovery-and-
    // config-parse the cache exists to avoid, and holding the map across it would
    // stall every other repo's scan behind it.
    let handle = backend.open(path).ok()?;
    let mut handles = lock_worktree_scan_handles(handles);
    // Closing can revoke the lease while discovery runs. Check under the same
    // lock used for ownership release so a late open cannot restore that owner.
    if !lease.is_open() {
        return None;
    }
    // A diff load and the scanner can open the same checkout concurrently.
    let now = handles.tick();
    if let Some(entry) = handles.entries.get_mut(&key) {
        entry.last_used = now;
        entry.owners.insert(owner);
        return Some(Arc::clone(&entry.handle));
    }
    while handles.entries.len() >= WORKTREE_SCAN_HANDLE_LIMIT && handles.evict_one_for(owner) {}
    let last_used = handles.tick();
    handles.entries.insert(
        key,
        WorktreeScanHandle {
            handle: Arc::clone(&handle),
            owners: FxHashSet::from_iter([owner]),
            last_used,
        },
    );
    Some(handle)
}

/// A handle on `repo`'s linked worktree at `path`, from the scan's cache.
/// Errors when `path` is not one of `repo`'s linked worktrees.
pub(super) fn linked_worktree_handle(
    backend: &dyn GitBackend,
    lease: &super::WorktreeCacheLease,
    repo: &dyn GitRepository,
    path: &Path,
) -> Result<Arc<dyn GitRepository>, Error> {
    let path = gitcomet_core::domain::normalize_worktree_path(path);
    let canonical = canonicalize_or_original(path.clone());
    // Git lists canonical paths; a caller may hold another spelling.
    let names_path = |candidate: &Path| {
        gitcomet_core::domain::normalize_worktree_path(candidate) == path
            || canonicalize_or_original(candidate.to_path_buf()) == canonical
    };
    let linked = !names_path(&repo.spec().workdir)
        && repo
            .list_worktrees(&gitcomet_core::services::CancellationToken::new())?
            .iter()
            .any(|worktree| names_path(&worktree.path));
    if !linked {
        return Err(Error::new(ErrorKind::Backend(format!(
            "{} is not a linked worktree of this repository",
            path.display()
        ))));
    }
    worktree_scan_handle(worktree_scan_handles(), backend, lease, &path).ok_or_else(|| {
        Error::new(ErrorKind::Backend(format!(
            "Could not open the worktree at {}",
            path.display()
        )))
    })
}

/// Drops this repo's handles for worktrees the current scan did not walk — ones
/// that have been pruned, removed, or unmounted since the last scan. Called with
/// the paths the scan actually saw, so a repo never accumulates handles for
/// worktrees that no longer exist.
fn retain_worktree_scan_handles(
    handles: &Mutex<WorktreeScanHandles>,
    owner: RepoLifetime,
    seen: &[PathBuf],
) {
    let seen: FxHashSet<_> = seen
        .iter()
        .map(|path| canonicalize_or_original(path.clone()))
        .collect();
    lock_worktree_scan_handles(handles)
        .entries
        .retain(|path, entry| {
            if !seen.contains(path) {
                entry.owners.remove(&owner);
            }
            !entry.owners.is_empty()
        });
}

fn forget_worktree_scan_handle(handles: &Mutex<WorktreeScanHandles>, path: &Path) {
    lock_worktree_scan_handles(handles)
        .entries
        .remove(&canonicalize_or_original(path.to_path_buf()));
}

/// Drops every scan handle, e.g. after a fetch, so none keeps old packs mapped.
pub(in crate::store) fn release_all_worktree_scan_handles() {
    lock_worktree_scan_handles(worktree_scan_handles())
        .entries
        .clear();
}

pub(in crate::store) fn release_worktree_scan_handles(owner: RepoLifetime) {
    retain_worktree_scan_handles(worktree_scan_handles(), owner, &[]);
}

/// Whether `worktree_path` is the worktree this tab already has open — those
/// changes belong in the pinned working-tree row, not in a linked-worktree one.
///
/// `own_workdir` is canonicalized: the spec's workdir goes through
/// `canonicalize_path` when the repo is opened. Git reports the path it recorded
/// verbatim, so the two can spell the same directory differently — `/tmp/x`
/// against `/private/tmp/x` on macOS, or either side of a symlinked repo root on
/// any platform. A raw comparison misses that, and this tab's own changes come
/// back a second time as a duplicate row anchored on HEAD. The cheap equality is
/// tried first so the common case costs no syscall.
fn is_own_worktree(worktree_path: &Path, own_workdir: &Path) -> bool {
    worktree_path == own_workdir
        || canonicalize_or_original(worktree_path.to_path_buf()) == own_workdir
}

/// Counts always; the file lists only when this is the worktree the details pane
/// is showing. The counts are derived before the lists are dropped, so a summary
/// without files still reports exactly what the row renders.
fn worktree_dirty_summary(
    worktree: Worktree,
    status: RepoStatus,
    keep_files: bool,
    line_stats: gitcomet_core::domain::UncommittedLineStats,
) -> WorktreeDirtySummary {
    let (added, modified, deleted) = count_file_statuses(&status.unstaged);
    let (staged_added, staged_modified, staged_deleted) = count_file_statuses(&status.staged);
    let (staged, unstaged) = if keep_files {
        (
            Arc::unwrap_or_clone(status.staged),
            Arc::unwrap_or_clone(status.unstaged),
        )
    } else {
        (Vec::new(), Vec::new())
    };
    WorktreeDirtySummary {
        path: worktree.path,
        head: worktree.head,
        branch: worktree.branch,
        detached: worktree.detached,
        added: added + staged_added,
        modified: modified + staged_modified,
        deleted: deleted + staged_deleted,
        staged,
        unstaged,
        line_stats,
    }
}

pub(super) fn schedule_load_submodules(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    cancellation: CancellationToken,
) {
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::SubmodulesLoaded {
                    repo_id,
                    result: repo.list_submodules(&cancellation),
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::SubmodulesLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(in crate::store::effects) mod explorer_listing;

pub(super) fn schedule_load_large_file_support(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    cancellation: CancellationToken,
) {
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::LargeFileSupportLoaded {
                    repo_id,
                    result: repo.large_file_support(&cancellation),
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::LargeFileSupportLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_lfs_locks(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    cancellation: CancellationToken,
) {
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::LfsLocksLoaded {
                    repo_id,
                    result: repo.lfs_locks(&cancellation),
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::LfsLocksLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_annex_unused(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    cancellation: CancellationToken,
) {
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            let result = repo.annex_unused(&cancellation);
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::AnnexUnusedLoaded { repo_id, result }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::AnnexUnusedLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_annex_whereis(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    key: String,
    cancellation: CancellationToken,
) {
    let missing_key = key.clone();
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            let result = repo.annex_whereis(&key, &cancellation);
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::AnnexWhereisLoaded {
                    repo_id,
                    key,
                    result,
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::AnnexWhereisLoaded {
                    repo_id,
                    key: missing_key,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_file_browser(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    source: gitcomet_core::domain::FileSource,
    cancellation: CancellationToken,
    options: explorer_listing::Options,
) {
    let source_for_err = source.clone();
    let cancellation_for_err = cancellation.clone();
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            let result = match &source {
                gitcomet_core::domain::FileSource::WorkingDirectory => {
                    super::super::repository_requirement::check(
                        repo.as_ref(),
                        gitcomet_core::domain::RepositoryRequirement::WorkingTree,
                    )
                    .and_then(|()| repo.list_worktree_files())
                    .and_then(|entries| {
                        explorer_listing::augment(
                            &repo.spec().workdir,
                            entries,
                            options,
                            &cancellation,
                        )
                    })
                }
                gitcomet_core::domain::FileSource::Commit(commit_id) => {
                    repo.list_tree_files_at_commit(commit_id)
                }
                gitcomet_core::domain::FileSource::Branch(_name) => {
                    Err(Error::new(gitcomet_core::error::ErrorKind::Backend(
                        "branch file listing is not yet implemented".to_string(),
                    )))
                }
            };
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::FileBrowserLoaded {
                    cancellation: Some(cancellation),
                    repo_id,
                    source,
                    result,
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::FileBrowserLoaded {
                    cancellation: Some(cancellation_for_err),
                    repo_id,
                    source: source_for_err,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_rebase_state(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    cancellation: CancellationToken,
) {
    spawn_detached_with_repo_or_else(
        executor,
        "load-rebase-state",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::RebaseStateLoaded {
                    repo_id,
                    result: repo.sequencer_state(&cancellation),
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::RebaseStateLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_rebase_and_merge_state(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    cancellation: CancellationToken,
) {
    spawn_detached_with_repo_or_else(
        executor,
        "load-rebase-and-merge-state",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::RebaseStateLoaded {
                    repo_id,
                    result: repo.sequencer_state(&cancellation),
                }),
            );
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::MergeCommitMessageLoaded {
                    repo_id,
                    result: repo.merge_commit_message(&cancellation),
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::RebaseStateLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::MergeCommitMessageLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_merge_commit_message(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    cancellation: CancellationToken,
) {
    spawn_detached_with_repo_or_else(
        executor,
        "load-merge-commit-message",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::MergeCommitMessageLoaded {
                    repo_id,
                    result: repo.merge_commit_message(&cancellation),
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::MergeCommitMessageLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_load_hover_commit_message(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    commit_id: gitcomet_core::domain::CommitId,
) {
    let fallback_id = commit_id.clone();
    spawn_detached_with_repo_or_else(
        executor,
        "load-hover-commit-message",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            // `commit_messages` is message-only by design, so hovering a row
            // does not pay for the tree diff `commit_details` computes.
            let result = repo
                .commit_messages(std::slice::from_ref(&commit_id))
                .and_then(|mut messages| {
                    if messages.is_empty() {
                        Err(Error::new(ErrorKind::Backend(format!(
                            "no message for commit {}",
                            commit_id.as_ref()
                        ))))
                    } else {
                        Ok(messages.remove(0))
                    }
                });
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::HoverCommitMessageLoaded {
                    repo_id,
                    commit_id,
                    result,
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::HoverCommitMessageLoaded {
                    repo_id,
                    commit_id: fallback_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn schedule_verify_commit_signatures(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    epoch: u64,
    batch: u64,
    cancellation: CancellationToken,
    commit_ids: std::sync::Arc<[gitcomet_core::domain::CommitId]>,
    formats: gitcomet_core::domain::SignatureFormats,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::CommitSignaturesVerified {
                repo_id,
                epoch,
                batch,
                result: repo.verify_commit_signatures(&commit_ids, formats, &cancellation),
            }),
        );
    });
}

/// Resolve a possibly abbreviated reference and load its details in one call.
///
/// `commit_details` runs the reference through `rev-parse`, so this answers
/// "does it exist, and is it unambiguous?" as a side effect of the load the
/// details pane needs anyway.
pub(super) fn schedule_resolve_commit_for_reveal(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    reference: gitcomet_core::domain::CommitId,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::CommitRevealResolved {
                repo_id,
                reference: reference.clone(),
                result: repo.commit_details(&reference),
            }),
        );
    });
}

/// Resolve a reference for the Reveal Commit dialog's preview row.
///
/// `resolve_commit` skips the parent diff `commit_details` pays for, because
/// this runs while the user is still typing.
pub(super) fn schedule_resolve_commit_lookup(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    reference: gitcomet_core::domain::CommitId,
    purpose: crate::model::CommitLookupPurpose,
    request: u64,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::CommitLookupResolved {
                repo_id,
                reference: reference.clone(),
                request,
                purpose,
                result: repo.resolve_commit(&reference),
            }),
        );
    });
}

#[allow(clippy::too_many_arguments)]
pub(super) fn schedule_load_range_files(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    from: gitcomet_core::domain::CommitId,
    to: Option<gitcomet_core::domain::CommitId>,
    options: gitcomet_core::services::ComparisonOptions,
    request: u64,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        // History and hosted views share this comparison service.
        let result = repo.compare_files(&from, to.as_ref(), &options, &CancellationToken::new());
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::RangeFilesLoaded {
                repo_id,
                from: from.clone(),
                to: to.clone(),
                request,
                result,
            }),
        );
    });
}

pub(super) fn schedule_load_squash_message_preview(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    oldest: gitcomet_core::domain::CommitId,
    head: gitcomet_core::domain::CommitId,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::SquashMessagePreviewLoaded {
                repo_id,
                oldest: oldest.clone(),
                head: head.clone(),
                result: repo.squash_message_preview(&oldest, &head),
            }),
        );
    });
}

/// Payload for scheduling a squash-via-rebase setup load. Bundled so the
/// scheduler stays within the argument-count budget and the fields travel
/// together into the resulting `SquashRebaseSetupLoaded` message.
pub(super) struct SquashRebaseSetupRequest {
    pub base: gitcomet_core::domain::CommitId,
    pub actual_head: gitcomet_core::domain::CommitId,
    pub selected_ids: Vec<gitcomet_core::domain::CommitId>,
    pub reword_id: gitcomet_core::domain::CommitId,
    pub message: String,
    pub count: usize,
}

pub(super) fn schedule_load_squash_rebase_setup(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    request: SquashRebaseSetupRequest,
) {
    let SquashRebaseSetupRequest {
        base,
        actual_head,
        selected_ids,
        reword_id,
        message,
        count,
    } = request;
    let base_str = base.as_ref().to_string();
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = repo.list_commits_for_interactive_rebase(&base_str);
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::SquashRebaseSetupLoaded {
                repo_id,
                base: base_str,
                actual_head,
                selected_ids,
                reword_id,
                message,
                count,
                result,
            }),
        );
    });
}

pub(super) fn schedule_open_file_at_commit(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    commit_id: gitcomet_core::domain::CommitId,
    path: std::path::PathBuf,
    content_preview: bool,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        // Resolve the file's name in the target commit (it may differ from the
        // path we hold due to a rename), then open content there. On failure or
        // when no mapping is found, fall back to the path as-is.
        let resolved = repo
            .resolve_file_path_at_commit(&path, &commit_id)
            .ok()
            .flatten()
            .unwrap_or(path);
        let message = if content_preview {
            Msg::OpenFileContent {
                repo_id,
                source: gitcomet_core::domain::FileSource::Commit(commit_id),
                path: resolved,
            }
        } else {
            Msg::SelectDiff {
                repo_id,
                target: gitcomet_core::domain::DiffTarget::commit(commit_id, resolved),
            }
        };
        send_or_log(&msg_tx, message);
    });
}

pub(super) fn schedule_open_file_at_commit_parent(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    commit_id: gitcomet_core::domain::CommitId,
    path: std::path::PathBuf,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        match repo.commit_details(&commit_id) {
            Ok(details) => {
                if let Some(parent) = details.parent_ids.first() {
                    // Resolve the file's name in the parent (it may differ from
                    // the path we hold due to a rename), mirroring
                    // `schedule_open_file_at_commit`. Falls back to the path
                    // as-is on failure or when no mapping is found.
                    let resolved = repo
                        .resolve_file_path_at_commit(&path, parent)
                        .ok()
                        .flatten()
                        .unwrap_or(path);
                    send_or_log(
                        &msg_tx,
                        Msg::OpenFileContent {
                            repo_id,
                            source: gitcomet_core::domain::FileSource::Commit(parent.clone()),
                            path: resolved,
                        },
                    );
                }
                // Root commit: no prior revision to open.
            }
            Err(e) => {
                // Could not resolve the commit's parent (e.g. backend/object
                // error). The affordance was shown, so surface the failure
                // instead of silently doing nothing.
                send_or_log(
                    &msg_tx,
                    Msg::ReportError {
                        repo_id: Some(repo_id),
                        message: format!("Could not open file at parent commit: {e}"),
                    },
                );
            }
        }
    });
}

pub(super) fn schedule_load_recent_commit_messages(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    limit: usize,
    request_rev: u64,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::RecentCommitMessagesLoaded {
                repo_id,
                request_rev,
                result: repo.recent_commit_messages(limit),
            }),
        );
    });
}

/// The user's encoding choice for `path`, when it is the open file.
pub(super) fn file_encoding_override(
    thread_state: &RwLock<Arc<AppState>>,
    repo_id: RepoId,
    path: &Path,
) -> Option<gitcomet_core::text_format::TextEncoding> {
    let state = thread_state.read().unwrap_or_else(|e| e.into_inner());
    state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)?
        .diff_state
        .text_override_for(path)?
        .encoding
}

pub(super) fn schedule_load_diff(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    target: DiffTarget,
    encoding: Option<gitcomet_core::text_format::TextEncoding>,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        // UI consumes this parsed diff through paged/lazy row adapters.
        let result = repo.diff_parsed(&target, encoding, &CancellationToken::new());
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::DiffLoaded {
                repo_id,
                target,
                result,
            }),
        );
    });
}

pub(super) fn schedule_load_diff_file(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    target: DiffTarget,
    encoding: Option<gitcomet_core::text_format::TextEncoding>,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = repo.diff_file_text(&target, encoding, &CancellationToken::new());
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::DiffFileLoaded {
                repo_id,
                target,
                result,
            }),
        );
    });
}

pub(super) fn schedule_load_diff_preview_text_file(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    target: DiffTarget,
    side: DiffPreviewTextSide,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = repo.diff_preview_text_file(
            &target,
            side,
            &gitcomet_core::services::CancellationToken::new(),
        );
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::DiffPreviewTextFileLoaded {
                repo_id,
                target,
                side,
                result,
            }),
        );
    });
}

pub(super) fn schedule_load_submodule_summary(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    target: DiffTarget,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = repo
            .submodule_diff_summary(&target, &gitcomet_core::services::CancellationToken::new());
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::SubmoduleSummaryLoaded {
                repo_id,
                target,
                result,
            }),
        );
    });
}

pub(super) fn schedule_load_inline_submodule_selected_diff(
    executor: &TaskExecutor,
    backend: Arc<dyn GitBackend>,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    inline_rev: u64,
    selected: Option<(PathBuf, DiffTarget, u64)>,
) {
    let Some((submodule_repo_path, target, current_rev)) = selected else {
        return;
    };
    if current_rev != inline_rev {
        return;
    }

    executor.spawn(move || {
        let result = backend.open(&submodule_repo_path).and_then(|repo| {
            repo.diff_parsed(
                &target,
                None,
                &gitcomet_core::services::CancellationToken::new(),
            )
        });
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::InlineSubmoduleDiffLoaded {
                repo_id,
                inline_rev,
                target,
                result,
            }),
        );
    });
}

pub(super) fn schedule_load_inline_submodule_selected_diff_file(
    executor: &TaskExecutor,
    backend: Arc<dyn GitBackend>,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    inline_rev: u64,
    selected: Option<(PathBuf, DiffTarget, u64)>,
) {
    let Some((submodule_repo_path, target, current_rev)) = selected else {
        return;
    };
    if current_rev != inline_rev {
        return;
    }

    executor.spawn(move || {
        let result = backend.open(&submodule_repo_path).and_then(|repo| {
            repo.diff_file_text(
                &target,
                None,
                &gitcomet_core::services::CancellationToken::new(),
            )
        });
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::InlineSubmoduleDiffFileLoaded {
                repo_id,
                inline_rev,
                target,
                result,
            }),
        );
    });
}

pub(super) fn schedule_load_inline_submodule_selected_diff_file_image(
    executor: &TaskExecutor,
    backend: Arc<dyn GitBackend>,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    inline_rev: u64,
    selected: Option<(PathBuf, DiffTarget, u64)>,
) {
    let Some((submodule_repo_path, target, current_rev)) = selected else {
        return;
    };
    if current_rev != inline_rev {
        return;
    }

    executor.spawn(move || {
        let result = backend.open(&submodule_repo_path).and_then(|repo| {
            repo.diff_file_image(&target, &gitcomet_core::services::CancellationToken::new())
        });
        send_or_log(
            &msg_tx,
            Msg::Internal(
                crate::msg::InternalMsg::InlineSubmoduleDiffFileImageLoaded {
                    repo_id,
                    inline_rev,
                    target,
                    result,
                },
            ),
        );
    });
}

pub(super) fn schedule_load_diff_file_image(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    target: DiffTarget,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result =
            repo.diff_file_image(&target, &gitcomet_core::services::CancellationToken::new());
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::DiffFileImageLoaded {
                repo_id,
                target,
                result,
            }),
        );
    });
}

pub(super) fn schedule_load_selected_diff(
    executor: &TaskExecutor,
    slots: &super::super::executor::SelectedDiffSlots,
    repos: &RepositoryRuntimes,
    thread_state: Arc<RwLock<Arc<AppState>>>,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    selection: (DiffTarget, u64),
    cancellation: CancellationToken,
    options: SelectedDiffLoadOptions,
) {
    let (target, target_rev) = selection;
    let encoding = target
        .file_path()
        .and_then(|path| file_encoding_override(&thread_state, repo_id, path));
    let guard = SelectedDiffLoadGuard::new(thread_state, repo_id, target.clone(), target_rev);
    if options.load_text_attributes
        && let Some(path) = target.file_path().map(Path::to_path_buf)
    {
        let target = target.clone();
        spawn_with_selected_diff_guard(
            executor,
            &slots.text_attributes,
            "selected_diff_text_attributes",
            repos,
            repo_id,
            msg_tx.clone(),
            guard.clone(),
            move |repo, msg_tx, guard| {
                let result = repo.text_attributes(&path);
                if !guard.is_current() {
                    return;
                }
                send_or_log(
                    &msg_tx,
                    Msg::Internal(crate::msg::InternalMsg::TextAttributesLoaded {
                        repo_id,
                        target,
                        result,
                    }),
                );
            },
        );
    }
    if options.load_patch_diff {
        let target = target.clone();
        let msg_tx = msg_tx.clone();
        let guard = guard.clone();
        let cancellation = cancellation.clone();
        spawn_with_selected_diff_guard(
            executor,
            &slots.patch,
            "selected_diff_patch",
            repos,
            repo_id,
            msg_tx,
            guard,
            move |repo, msg_tx, guard| {
                // UI consumes this parsed diff through paged/lazy row adapters.
                let result = repo.diff_parsed(&target, encoding, &cancellation);
                if !guard.is_current() {
                    return;
                }
                send_or_log(
                    &msg_tx,
                    Msg::Internal(crate::msg::InternalMsg::DiffLoaded {
                        repo_id,
                        target,
                        result,
                    }),
                );
            },
        );
    }
    if options.load_file_text {
        let target = target.clone();
        let cancellation = cancellation.clone();
        spawn_with_selected_diff_guard(
            executor,
            &slots.file_text,
            "selected_diff_file_text",
            repos,
            repo_id,
            msg_tx.clone(),
            guard.clone(),
            move |repo, msg_tx, guard| {
                let result = repo.diff_file_text(&target, encoding, &cancellation);
                if !guard.is_current() {
                    return;
                }
                send_or_log(
                    &msg_tx,
                    Msg::Internal(crate::msg::InternalMsg::DiffFileLoaded {
                        repo_id,
                        target,
                        result,
                    }),
                );
            },
        );
    }
    if options.load_submodule_summary {
        let target = target.clone();
        let cancellation = cancellation.clone();
        spawn_with_selected_diff_guard(
            executor,
            &slots.submodule_summary,
            "selected_diff_submodule_summary",
            repos,
            repo_id,
            msg_tx.clone(),
            guard.clone(),
            move |repo, msg_tx, guard| {
                let result = repo.submodule_diff_summary(&target, &cancellation);
                if !guard.is_current() {
                    return;
                }
                send_or_log(
                    &msg_tx,
                    Msg::Internal(crate::msg::InternalMsg::SubmoduleSummaryLoaded {
                        repo_id,
                        target,
                        result,
                    }),
                );
            },
        );
    }
    if options.load_file_image {
        let target = target.clone();
        let cancellation = cancellation.clone();
        spawn_with_selected_diff_guard(
            executor,
            &slots.file_image,
            "selected_diff_file_image",
            repos,
            repo_id,
            msg_tx.clone(),
            guard.clone(),
            move |repo, msg_tx, guard| {
                let result = repo.diff_file_image(&target, &cancellation);
                if !guard.is_current() {
                    return;
                }
                send_or_log(
                    &msg_tx,
                    Msg::Internal(crate::msg::InternalMsg::DiffFileImageLoaded {
                        repo_id,
                        target,
                        result,
                    }),
                );
            },
        );
    }
    if let Some(side) = options.preview_text_side {
        let target = target.clone();
        let cancellation = cancellation.clone();
        spawn_with_selected_diff_guard(
            executor,
            &slots.preview_text,
            "selected_diff_preview_text",
            repos,
            repo_id,
            msg_tx.clone(),
            guard.clone(),
            move |repo, msg_tx, guard| {
                let result = repo.diff_preview_text_file(&target, side, &cancellation);
                if !guard.is_current() {
                    return;
                }
                send_or_log(
                    &msg_tx,
                    Msg::Internal(crate::msg::InternalMsg::DiffPreviewTextFileLoaded {
                        repo_id,
                        target,
                        side,
                        result,
                    }),
                );
            },
        );
    }
}

/// Loads the full `%B` message of every selected cherry-pick source commit.
/// Rewording stays unavailable if any lookup fails: falling back to the
/// subject-only seed would make saving the dialog destructive.
pub(super) fn schedule_load_interactive_cherry_pick_messages(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    ids: Vec<String>,
) {
    let fallback_ids = ids.clone();
    spawn_detached_with_repo_or_else(
        executor,
        "load-interactive-cherry-pick-messages",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            let commit_ids = ids
                .iter()
                .map(|id| gitcomet_core::domain::CommitId(id.clone().into()))
                .collect::<Vec<_>>();
            let result = repo
                .topologically_order_commits(&commit_ids)
                .and_then(|ordered_ids| {
                    repo.commit_messages(&ordered_ids).map(|messages| {
                        ordered_ids
                            .into_iter()
                            .map(|id| id.as_ref().to_string())
                            .zip(messages)
                            .collect()
                    })
                });
            send_or_log(
                &msg_tx,
                Msg::Internal(
                    crate::msg::InternalMsg::InteractiveCherryPickMessagesLoaded {
                        repo_id,
                        requested_ids: ids,
                        result,
                    },
                ),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(
                    crate::msg::InternalMsg::InteractiveCherryPickMessagesLoaded {
                        repo_id,
                        requested_ids: fallback_ids,
                        result: Err(Error::new(ErrorKind::Backend(
                            "repository unavailable while loading cherry-pick commit messages"
                                .to_string(),
                        ))),
                    },
                ),
            );
        },
    );
}

pub(super) fn schedule_load_interactive_rebase_setup(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    base: String,
) {
    let base_for_call = base.clone();
    let base_for_err = base.clone();
    spawn_detached_with_repo_or_else(
        executor,
        "load-interactive-rebase-setup",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            let result = repo.list_commits_for_interactive_rebase(&base_for_call);
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::InteractiveRebaseSetupLoaded {
                    repo_id,
                    base,
                    result,
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(crate::msg::InternalMsg::InteractiveRebaseSetupLoaded {
                    repo_id,
                    base: base_for_err,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

#[cfg(test)]
mod worktree_dirty_tests;

#[cfg(test)]
mod worktree_scan_handle_tests;

#[cfg(test)]
mod worktree_dirty_files_tests;
