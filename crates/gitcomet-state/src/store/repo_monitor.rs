use crate::model::RepoId;
use crate::msg::{Msg, RepoExternalChange, RepoWatchDegradedReason};
use gitcomet_core::services::{GitBackend, WorktreeIgnoreMatcher, WorktreePathKind};
use notify::event::{AccessKind, AccessMode, EventKindMask};
use notify::{Config as NotifyConfig, RecommendedWatcher, RecursiveMode, Watcher};
use rustc_hash::{FxHashMap, FxHashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use super::repo_load_trace;
use super::send_diagnostics::{SendFailureKind, panic_payload_to_string, send_or_log};
use super::worker_channel::StoreWorkerSender;

enum MonitorMsg {
    Event(notify::Result<notify::Event>),
    Stop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MonitorFailureKind {
    Start,
    Stop,
    Join,
}

static REPO_MONITOR_START_FAILURES: AtomicU64 = AtomicU64::new(0);
static REPO_MONITOR_STOP_FAILURES: AtomicU64 = AtomicU64::new(0);
static REPO_MONITOR_JOIN_FAILURES: AtomicU64 = AtomicU64::new(0);
static REPO_MONITOR_IGNORE_LOOKUP_REQUESTS: AtomicU64 = AtomicU64::new(0);
static REPO_MONITOR_IGNORE_LOOKUP_CACHE_HITS: AtomicU64 = AtomicU64::new(0);
static REPO_MONITOR_IGNORE_LOOKUP_CACHE_MISSES: AtomicU64 = AtomicU64::new(0);
static REPO_MONITOR_IGNORE_LOOKUP_FALLBACKS: AtomicU64 = AtomicU64::new(0);
static REPO_MONITOR_IGNORE_LOOKUP_TOTAL_NANOS: AtomicU64 = AtomicU64::new(0);
static REPO_MONITOR_IGNORE_LOOKUP_MAX_NANOS: AtomicU64 = AtomicU64::new(0);

fn duration_nanos_saturating(duration: Duration) -> u64 {
    duration.as_nanos().min(u64::MAX as u128) as u64
}

fn record_ignore_lookup_cache_outcome(hit: bool) {
    REPO_MONITOR_IGNORE_LOOKUP_REQUESTS.fetch_add(1, Ordering::Relaxed);
    if hit {
        REPO_MONITOR_IGNORE_LOOKUP_CACHE_HITS.fetch_add(1, Ordering::Relaxed);
    } else {
        REPO_MONITOR_IGNORE_LOOKUP_CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
    }
}

fn record_ignore_lookup_latency(duration: Duration, used_fallback: bool) {
    let nanos = duration_nanos_saturating(duration);
    REPO_MONITOR_IGNORE_LOOKUP_TOTAL_NANOS.fetch_add(nanos, Ordering::Relaxed);
    if used_fallback {
        REPO_MONITOR_IGNORE_LOOKUP_FALLBACKS.fetch_add(1, Ordering::Relaxed);
    }

    let mut current = REPO_MONITOR_IGNORE_LOOKUP_MAX_NANOS.load(Ordering::Relaxed);
    while nanos > current {
        match REPO_MONITOR_IGNORE_LOOKUP_MAX_NANOS.compare_exchange_weak(
            current,
            nanos,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(observed) => current = observed,
        }
    }
}

fn monitor_failure_counter(kind: MonitorFailureKind) -> &'static AtomicU64 {
    match kind {
        MonitorFailureKind::Start => &REPO_MONITOR_START_FAILURES,
        MonitorFailureKind::Stop => &REPO_MONITOR_STOP_FAILURES,
        MonitorFailureKind::Join => &REPO_MONITOR_JOIN_FAILURES,
    }
}

fn record_monitor_failure(
    kind: MonitorFailureKind,
    context: &'static str,
    detail: impl std::fmt::Display,
) {
    let count = monitor_failure_counter(kind).fetch_add(1, Ordering::Relaxed) + 1;
    // This runs on threads with no unwind guard (see
    // process::write_stderr_line).
    gitcomet_core::process::write_stderr_line(format_args!(
        "gitcomet-state: repo monitor failure ({kind:?}) in {context}: {detail}; total_failures={count}"
    ));
}

fn send_stop_or_log(tx: &mpsc::Sender<MonitorMsg>, repo_id: RepoId, context: &'static str) {
    if let Err(error) = tx.send(MonitorMsg::Stop) {
        record_monitor_failure(
            MonitorFailureKind::Stop,
            context,
            format!("repo_id={repo_id:?}; send failed: {error}"),
        );
    }
}

fn send_watcher_event_or_log(
    repo_id: RepoId,
    tx: &mpsc::Sender<MonitorMsg>,
    event: notify::Result<notify::Event>,
    monitor_enabled: &AtomicBool,
) -> bool {
    if !monitor_enabled.load(Ordering::Relaxed) {
        repo_load_trace::trace!("repo_monitor_drop_event_after_stop repo_id={:?}", repo_id);
        return false;
    }

    send_or_log(
        tx,
        MonitorMsg::Event(event),
        SendFailureKind::RepoMonitorMessage,
        "repo monitor watcher callback",
    );
    true
}

pub(super) fn join_monitor_or_log(
    join: thread::JoinHandle<()>,
    repo_id: RepoId,
    context: &'static str,
) {
    if let Err(error) = join.join() {
        record_monitor_failure(
            MonitorFailureKind::Join,
            context,
            format!(
                "repo_id={repo_id:?}; join failed: {}",
                panic_payload_to_string(error.as_ref())
            ),
        );
    }
}

fn spawn_monitor_join(repo_id: RepoId, join: thread::JoinHandle<()>, context: &'static str) {
    let thread_name = format!("gitcomet-repo-monitor-join-{}", repo_id.0);
    if let Err(error) = thread::Builder::new().name(thread_name).spawn(move || {
        repo_load_trace::trace!(
            "repo_monitor_async_join_start repo_id={:?} context={}",
            repo_id,
            context
        );
        join_monitor_or_log(join, repo_id, context);
        repo_load_trace::trace!(
            "repo_monitor_async_join_finish repo_id={:?} context={}",
            repo_id,
            context
        );
    }) {
        record_monitor_failure(
            MonitorFailureKind::Join,
            context,
            format!("repo_id={repo_id:?}; failed to spawn async join thread: {error}"),
        );
    }
}

#[cfg(test)]
pub(super) fn monitor_failure_count(kind: MonitorFailureKind) -> u64 {
    monitor_failure_counter(kind).load(Ordering::Relaxed)
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct RepoMonitorIgnoreLookupStats {
    pub(super) request_count: u64,
    pub(super) cache_hits: u64,
    pub(super) cache_misses: u64,
    pub(super) fallback_count: u64,
    pub(super) average_lookup_nanos: u64,
    pub(super) max_lookup_nanos: u64,
}

#[cfg(test)]
pub(super) fn repo_monitor_ignore_lookup_stats() -> RepoMonitorIgnoreLookupStats {
    let request_count = REPO_MONITOR_IGNORE_LOOKUP_REQUESTS.load(Ordering::Relaxed);
    let cache_hits = REPO_MONITOR_IGNORE_LOOKUP_CACHE_HITS.load(Ordering::Relaxed);
    let cache_misses = REPO_MONITOR_IGNORE_LOOKUP_CACHE_MISSES.load(Ordering::Relaxed);
    let fallback_count = REPO_MONITOR_IGNORE_LOOKUP_FALLBACKS.load(Ordering::Relaxed);
    let total_lookup_nanos = REPO_MONITOR_IGNORE_LOOKUP_TOTAL_NANOS.load(Ordering::Relaxed);
    let max_lookup_nanos = REPO_MONITOR_IGNORE_LOOKUP_MAX_NANOS.load(Ordering::Relaxed);
    let average_lookup_nanos = total_lookup_nanos.checked_div(cache_misses).unwrap_or(0);

    RepoMonitorIgnoreLookupStats {
        request_count,
        cache_hits,
        cache_misses,
        fallback_count,
        average_lookup_nanos,
        max_lookup_nanos,
    }
}

#[cfg(test)]
pub(super) fn record_stop_send_failure(repo_id: RepoId, context: &'static str) {
    let (tx, rx) = mpsc::channel::<MonitorMsg>();
    drop(rx);
    send_stop_or_log(&tx, repo_id, context);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DebouncedChange {
    pending: Option<RepoExternalChange>,
    first_event_at: Option<Instant>,
    last_event_at: Option<Instant>,
    debounce: Duration,
    max_delay: Duration,
}

impl DebouncedChange {
    fn new(debounce: Duration, max_delay: Duration) -> Self {
        Self {
            pending: None,
            first_event_at: None,
            last_event_at: None,
            debounce,
            max_delay,
        }
    }

    fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    fn push(&mut self, change: RepoExternalChange, now: Instant) -> Option<RepoExternalChange> {
        self.pending = Some(merge_change(self.pending.unwrap_or(change), change));
        self.first_event_at.get_or_insert(now);
        self.last_event_at = Some(now);
        self.take_if_max_delay_elapsed(now)
    }

    fn take_if_max_delay_elapsed(&mut self, now: Instant) -> Option<RepoExternalChange> {
        let first = self.first_event_at?;
        if now.duration_since(first) >= self.max_delay {
            self.take()
        } else {
            None
        }
    }

    fn next_timeout(&self, now: Instant) -> Option<Duration> {
        let (first, last) = (self.first_event_at?, self.last_event_at?);
        let due_by_debounce = last + self.debounce;
        let due_by_max = first + self.max_delay;
        let due = if due_by_debounce <= due_by_max {
            due_by_debounce
        } else {
            due_by_max
        };
        Some(due.saturating_duration_since(now))
    }

    fn take_if_due(&mut self, now: Instant) -> Option<RepoExternalChange> {
        if !self.is_pending() {
            return None;
        }
        let timeout = self.next_timeout(now).unwrap_or(Duration::from_secs(0));
        if timeout.is_zero() { self.take() } else { None }
    }

    fn take(&mut self) -> Option<RepoExternalChange> {
        let pending = self.pending.take();
        self.first_event_at = None;
        self.last_event_at = None;
        pending
    }
}

pub(super) struct RepoMonitorManager {
    handles: FxHashMap<RepoId, RepoMonitorHandle>,
}

impl RepoMonitorManager {
    pub(super) fn new() -> Self {
        Self {
            handles: FxHashMap::default(),
        }
    }

    pub(super) fn stop_all(&mut self) {
        for (repo_id, handle) in self.handles.drain() {
            stop_monitor_handle(repo_id, handle, "RepoMonitorManager::stop_all");
        }
    }

    pub(super) fn stop(&mut self, repo_id: RepoId) {
        let Some(handle) = self.handles.remove(&repo_id) else {
            return;
        };
        stop_monitor_handle(repo_id, handle, "RepoMonitorManager::stop");
    }

    pub(super) fn running_repo_ids(&self) -> Vec<RepoId> {
        self.handles.keys().copied().collect()
    }

    pub(super) fn is_running(&self, repo_id: RepoId) -> bool {
        self.handles
            .get(&repo_id)
            .is_some_and(|handle| handle.monitor_enabled.load(Ordering::Relaxed))
    }

    pub(super) fn start(
        &mut self,
        repo_id: RepoId,
        workdir: PathBuf,
        msg_tx: StoreWorkerSender,
        active_repo_id: Arc<AtomicU64>,
        backend: Arc<dyn GitBackend>,
    ) {
        let std::collections::hash_map::Entry::Vacant(entry) = self.handles.entry(repo_id) else {
            return;
        };
        let (monitor_tx, monitor_rx) = mpsc::channel::<MonitorMsg>();
        let monitor_tx_for_notify = monitor_tx.clone();
        let monitor_enabled = Arc::new(AtomicBool::new(true));
        let monitor_enabled_for_thread = Arc::clone(&monitor_enabled);
        let join = thread::spawn(move || {
            repo_monitor_thread(
                repo_id,
                workdir,
                msg_tx,
                monitor_rx,
                monitor_tx_for_notify,
                active_repo_id,
                monitor_enabled_for_thread,
                backend,
            )
        });
        entry.insert(RepoMonitorHandle {
            msg_tx: monitor_tx,
            join,
            monitor_enabled,
        });
    }

    #[cfg(test)]
    pub(super) fn insert_blocked_monitor_for_test(
        &mut self,
        repo_id: RepoId,
        release_rx: mpsc::Receiver<()>,
        exited_tx: mpsc::Sender<()>,
    ) -> Arc<AtomicBool> {
        let (monitor_tx, monitor_rx) = mpsc::channel::<MonitorMsg>();
        let monitor_enabled = Arc::new(AtomicBool::new(true));
        let join = thread::spawn(move || {
            let _ = monitor_rx.recv();
            let _ = release_rx.recv();
            let _ = exited_tx.send(());
        });
        self.handles.insert(
            repo_id,
            RepoMonitorHandle {
                msg_tx: monitor_tx,
                join,
                monitor_enabled: Arc::clone(&monitor_enabled),
            },
        );
        monitor_enabled
    }
}

struct RepoMonitorHandle {
    msg_tx: mpsc::Sender<MonitorMsg>,
    join: thread::JoinHandle<()>,
    monitor_enabled: Arc<AtomicBool>,
}

fn stop_monitor_handle(repo_id: RepoId, handle: RepoMonitorHandle, context: &'static str) {
    repo_load_trace::trace!(
        "repo_monitor_stop_requested repo_id={:?} context={}",
        repo_id,
        context
    );
    handle.monitor_enabled.store(false, Ordering::Relaxed);
    send_stop_or_log(&handle.msg_tx, repo_id, context);
    spawn_monitor_join(repo_id, handle.join, context);
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct IgnoreCacheKey {
    rel: PathBuf,
    is_dir_hint: Option<bool>,
}

const GITIGNORE_CACHE_MAX_ENTRIES: usize = 4_096;
const GITIGNORE_CACHE_TTL: Duration = Duration::from_secs(10 * 60);
const GITIGNORE_CACHE_PRUNE_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone)]
struct CachedIgnoreResult {
    ignored: bool,
    cached_at: Instant,
}

#[derive(Default)]
struct GitignoreRules {
    workdir: Option<PathBuf>,
    /// The backend used to rebuild `matcher`. Owned here because `reload` is
    /// called from event classification, deep in the monitor loop, where the
    /// backend is not otherwise in scope.
    ///
    /// `Option` only so this derives `Default`, which tests use to build rules
    /// that ignore nothing. `load` always supplies one, so the `None` arm of
    /// `reload` is unreachable in production.
    backend: Option<Arc<dyn GitBackend>>,
    matcher: Option<Box<dyn WorktreeIgnoreMatcher>>,
    cache: FxHashMap<IgnoreCacheKey, CachedIgnoreResult>,
    last_prune_at: Option<Instant>,
}

impl GitignoreRules {
    fn load(workdir: &Path, backend: Arc<dyn GitBackend>) -> Self {
        let mut rules = Self {
            backend: Some(backend),
            ..Self::default()
        };
        rules.reload(workdir);
        rules
    }

    fn reload(&mut self, workdir: &Path) {
        self.workdir = Some(workdir.to_path_buf());
        self.matcher = self
            .backend
            .as_ref()
            .and_then(|backend| backend.worktree_ignore_matcher(workdir).ok().flatten());
        self.cache.clear();
        self.last_prune_at = None;
    }

    fn is_cache_entry_fresh(now: Instant, entry: &CachedIgnoreResult) -> bool {
        now.saturating_duration_since(entry.cached_at) <= GITIGNORE_CACHE_TTL
    }

    fn prune_cache_if_due(&mut self, now: Instant) {
        let should_prune = match self.last_prune_at {
            Some(last_prune_at) => {
                now.saturating_duration_since(last_prune_at) >= GITIGNORE_CACHE_PRUNE_INTERVAL
                    || self.cache.len() > GITIGNORE_CACHE_MAX_ENTRIES
            }
            None => true,
        };

        if should_prune {
            self.prune_cache(now);
        }
    }

    fn prune_cache(&mut self, now: Instant) {
        self.cache
            .retain(|_, entry| Self::is_cache_entry_fresh(now, entry));

        if self.cache.len() > GITIGNORE_CACHE_MAX_ENTRIES {
            let mut keys_by_age: Vec<(IgnoreCacheKey, Instant)> = self
                .cache
                .iter()
                .map(|(key, entry)| (key.clone(), entry.cached_at))
                .collect();
            keys_by_age.sort_unstable_by_key(|(_, cached_at)| *cached_at);

            let overflow = keys_by_age
                .len()
                .saturating_sub(GITIGNORE_CACHE_MAX_ENTRIES);
            for (key, _) in keys_by_age.into_iter().take(overflow) {
                self.cache.remove(&key);
            }
        }

        self.last_prune_at = Some(now);
    }

    fn cache_get(&mut self, key: &IgnoreCacheKey, now: Instant) -> Option<bool> {
        let entry = self.cache.get(key)?;
        let (ignored, fresh) = (entry.ignored, Self::is_cache_entry_fresh(now, entry));

        if !fresh {
            self.cache.remove(key);
            return None;
        }

        Some(ignored)
    }

    fn cache_insert(&mut self, key: IgnoreCacheKey, ignored: bool, now: Instant) {
        self.cache.insert(
            key,
            CachedIgnoreResult {
                ignored,
                cached_at: now,
            },
        );
        self.prune_cache_if_due(now);
    }

    fn cached_ignore_lookup(&mut self, key: &IgnoreCacheKey, now: Instant) -> Option<bool> {
        let cached = self.cache_get(key, now);
        record_ignore_lookup_cache_outcome(cached.is_some());
        cached
    }

    fn resolve_uncached_ignore(&mut self, rel: &Path, is_dir_hint: Option<bool>) -> bool {
        let started_at = Instant::now();
        let kind = match is_dir_hint {
            Some(true) => WorktreePathKind::Directory,
            Some(false) => WorktreePathKind::File,
            None => WorktreePathKind::Unknown,
        };
        let (ignored, matcher_failed) = match self.matcher.as_mut() {
            Some(matcher) => match matcher.is_ignored(rel, kind) {
                Ok(ignored) => (ignored, false),
                // The backend matcher failed — treat as not-ignored (safe: may cause extra
                // refreshes, but never misses real changes).
                Err(_) => (false, true),
            },
            // No matcher available — treat as not-ignored.
            None => (false, true),
        };
        record_ignore_lookup_latency(started_at.elapsed(), matcher_failed);
        ignored
    }

    fn is_ignored_rel(&mut self, rel: &Path, is_dir_hint: Option<bool>) -> bool {
        if self.workdir.is_none() {
            return false;
        }

        let now = Instant::now();
        self.prune_cache_if_due(now);

        let key = IgnoreCacheKey {
            rel: rel.to_path_buf(),
            is_dir_hint,
        };
        if let Some(ignored) = self.cached_ignore_lookup(&key, now) {
            return ignored;
        }

        let ignored = self.resolve_uncached_ignore(rel, is_dir_hint);
        self.cache_insert(key, ignored, now);
        ignored
    }
}

/// Immediate subdirectories of the git dir that are deliberately never watched: `objects/` — a loose
/// object is written on nearly every git operation (high event churn) and its 256-way fanout on a
/// large repo would itself consume many inotify watches, yet object writes carry no UI-relevant
/// signal of their own (the accompanying ref/HEAD update is what the UI reacts to) — and `lfs/`,
/// git-LFS object storage which can be very large.
const GIT_DIR_WATCH_DENYLIST: [&str; 2] = ["objects", "lfs"];

fn is_denylisted_git_subdir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| GIT_DIR_WATCH_DENYLIST.contains(&name))
}

/// Watches the git directory for the state changes the UI reacts to (commit, checkout, fetch,
/// merge/rebase, stage), while excluding the high-churn denylist (`objects/`, `lfs/`). The git-dir
/// root is watched non-recursively (HEAD, index, packed-refs, `*_HEAD`, `FETCH_HEAD`) and every
/// other immediate subdirectory recursively — `refs/`, `logs/`, `info/`, and also in-progress
/// operation state like `rebase-merge/`, `rebase-apply/`, `sequencer/`, plus `worktrees/`,
/// `modules/`. Using a *denylist* of churny dirs (rather than an allowlist of state dirs) keeps new
/// or uncommon git state dirs covered without having to enumerate them. Subdirectories that appear
/// later are picked up by [`watch_created_git_subdirs`]. Excluding `objects/` also removes the
/// spurious refreshes its writes used to trigger.
fn setup_git_dir_watch(
    watcher: &mut RecommendedWatcher,
    git_dir: &Path,
    workdir: &Path,
    repo_id: RepoId,
) {
    if let Err(error) = watcher.watch(git_dir, RecursiveMode::NonRecursive) {
        record_monitor_failure(
            MonitorFailureKind::Start,
            "repo_monitor_thread watch git dir",
            format!(
                "repo_id={repo_id:?}, workdir={}, git_dir={}: {error}",
                workdir.display(),
                git_dir.display()
            ),
        );
    }

    let Ok(entries) = std::fs::read_dir(git_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_dir = entry
            .file_type()
            .map(|file_type| file_type.is_dir())
            .unwrap_or(false);
        if is_dir && !is_denylisted_git_subdir(&path) {
            // Best-effort; recursive when supported so nested ref/log/state files are observed.
            let _ = watcher
                .watch(&path, RecursiveMode::Recursive)
                .or_else(|_| watcher.watch(&path, RecursiveMode::NonRecursive));
        }
    }
}

/// Creates a fresh watcher and sets up every watch the monitor needs: the non-ignored worktree
/// tree and the git directory. Used both for the initial setup and to re-initiate watches after a
/// `.gitignore`/exclude change, where dropping the previous watcher releases its (now possibly
/// stale) inotify watches and this rebuilds the minimal set from the current ignore rules. Returns
/// `None` if the watcher cannot be created or the root worktree watch fails; otherwise the watcher
/// and the worktree setup outcome (so the caller can warn when source watching was disabled).
fn build_workdir_watcher(
    repo_id: RepoId,
    workdir: &Path,
    git_dir: Option<&Path>,
    gitignore: &mut GitignoreRules,
    watched_dirs: &mut FxHashSet<PathBuf>,
    monitor_tx: &mpsc::Sender<MonitorMsg>,
    monitor_enabled: &Arc<AtomicBool>,
) -> Option<(RecommendedWatcher, WatchSetupOutcome)> {
    let watcher = RecommendedWatcher::new(
        {
            let monitor_tx = monitor_tx.clone();
            let monitor_enabled = Arc::clone(monitor_enabled);
            move |res| {
                send_watcher_event_or_log(repo_id, &monitor_tx, res, monitor_enabled.as_ref());
            }
        },
        NotifyConfig::default().with_event_kinds(WATCHED_EVENT_KINDS),
    );

    let mut watcher: RecommendedWatcher = match watcher {
        Ok(w) => w,
        Err(error) => {
            record_monitor_failure(
                MonitorFailureKind::Start,
                "repo_monitor_thread initialize watcher",
                format!(
                    "repo_id={repo_id:?}, workdir={}: {error}",
                    workdir.display()
                ),
            );
            return None;
        }
    };

    // `setup_workdir_watch` repopulates `watched_dirs` with the freshly-watched worktree subdirs.
    let outcome = setup_workdir_watch(
        &mut watcher,
        watched_dirs,
        workdir,
        git_dir,
        gitignore,
        repo_id,
    );
    if outcome == WatchSetupOutcome::RootWatchFailed {
        return None;
    }

    if let Some(git_dir) = git_dir {
        setup_git_dir_watch(&mut watcher, git_dir, workdir, repo_id);
    }

    Some((watcher, outcome))
}

/// The user-facing degraded-watch reason for an outcome, or `None` when watching is healthy
/// (fully watched, or the root watch failed and the watcher is being discarded).
fn watch_degraded_reason(outcome: WatchSetupOutcome) -> Option<RepoWatchDegradedReason> {
    match outcome {
        #[cfg(any(target_os = "linux", test))]
        WatchSetupOutcome::WorktreeSubdirsSkipped { dir_count } => {
            Some(RepoWatchDegradedReason::TooManyFolders { dir_count })
        }
        WatchSetupOutcome::Watching { failed_dirs } if failed_dirs > 0 => {
            Some(RepoWatchDegradedReason::WatchLimitReached {
                unwatched_dirs: failed_dirs,
            })
        }
        WatchSetupOutcome::Watching { .. } | WatchSetupOutcome::RootWatchFailed => None,
    }
}

/// Pure transition logic for the degraded-watch warning: returns the reason exactly when the outcome
/// moves *into* a degraded state (skipped, or partially-watched), so the rare `.gitignore`-triggered
/// rebuilds and idle re-checks don't re-warn while it stays degraded; clears the flag on recovery.
fn watch_degraded_transition(
    previously_degraded: &mut bool,
    outcome: WatchSetupOutcome,
) -> Option<RepoWatchDegradedReason> {
    let reason = watch_degraded_reason(outcome);
    let should_warn = reason.is_some() && !*previously_degraded;
    *previously_degraded = reason.is_some();
    if should_warn { reason } else { None }
}

/// Surfaces the degraded-watch warning to the user when the worktree setup transitions into a
/// degraded state, and clears the flag when it recovers.
fn note_watch_outcome(
    msg_tx: &StoreWorkerSender,
    repo_id: RepoId,
    previously_degraded: &mut bool,
    outcome: WatchSetupOutcome,
) {
    if let Some(reason) = watch_degraded_transition(previously_degraded, outcome) {
        msg_tx.send_repo_monitor_or_log(
            Msg::RepoWatchDegraded { repo_id, reason },
            "repo monitor watch degraded",
        );
    }
}

/// While degraded (worktree over budget), re-check for recovery at most this often rather than on
/// every idle tick: each re-check rebuilds the backend matcher, which is wasteful to do every 30s
/// for a repo that stays over budget.
const DEGRADED_WATCH_RECHECK_INTERVAL: Duration = Duration::from_secs(120);

/// Whether a degraded-watch recovery re-check is due, given when one was last attempted.
fn recovery_recheck_due(last_attempt: Option<Instant>, now: Instant, interval: Duration) -> bool {
    match last_attempt {
        None => true,
        Some(last) => now.duration_since(last) >= interval,
    }
}

/// While the worktree is over budget (source watching disabled), a `.gitignore` edit in an
/// *unwatched* subdirectory can bring it back under budget without producing any event we can
/// observe. Called (throttled) on the idle tick: reloads the ignore rules and, only if the worktree
/// is now within budget, rebuilds the watcher so live watching resumes (repopulating `watched_dirs`).
/// Returns the rebuilt watcher + outcome on recovery, or `None` if still over budget / not currently
/// skipped. No-op off Linux, where the recursive watcher has no per-directory budget.
#[cfg(target_os = "linux")]
fn attempt_degraded_watch_recovery(
    watch_outcome: WatchSetupOutcome,
    repo_id: RepoId,
    workdir: &Path,
    git_dir: Option<&Path>,
    gitignore: &mut GitignoreRules,
    watched_dirs: &mut FxHashSet<PathBuf>,
    monitor_tx: &mpsc::Sender<MonitorMsg>,
    monitor_enabled: &Arc<AtomicBool>,
) -> Option<(RecommendedWatcher, WatchSetupOutcome)> {
    if !matches!(
        watch_outcome,
        WatchSetupOutcome::WorktreeSubdirsSkipped { .. }
    ) {
        return None;
    }
    // Reload first: we could not observe the (subdirectory) ignore edit that may have shrunk the
    // tree, so the current rules are potentially stale. The capped walk then bounds the cost of
    // re-checking while still over budget.
    gitignore.reload(workdir);
    let subdir_count = collect_watchable_dirs_capped(
        workdir,
        workdir,
        git_dir,
        gitignore,
        MAX_WORKTREE_WATCH_DIRS,
    )
    .len()
    .saturating_sub(1);
    if subdir_count > MAX_WORKTREE_WATCH_DIRS {
        return None;
    }
    build_workdir_watcher(
        repo_id,
        workdir,
        git_dir,
        gitignore,
        watched_dirs,
        monitor_tx,
        monitor_enabled,
    )
}

#[cfg(not(target_os = "linux"))]
fn attempt_degraded_watch_recovery(
    _watch_outcome: WatchSetupOutcome,
    _repo_id: RepoId,
    _workdir: &Path,
    _git_dir: Option<&Path>,
    _gitignore: &mut GitignoreRules,
    _watched_dirs: &mut FxHashSet<PathBuf>,
    _monitor_tx: &mpsc::Sender<MonitorMsg>,
    _monitor_enabled: &Arc<AtomicBool>,
) -> Option<(RecommendedWatcher, WatchSetupOutcome)> {
    None
}

fn repo_monitor_thread(
    repo_id: RepoId,
    workdir: PathBuf,
    msg_tx: StoreWorkerSender,
    monitor_rx: mpsc::Receiver<MonitorMsg>,
    monitor_tx: mpsc::Sender<MonitorMsg>,
    active_repo_id: Arc<AtomicU64>,
    monitor_enabled: Arc<AtomicBool>,
    backend: Arc<dyn GitBackend>,
) {
    let workdir = super::canonicalize_path(workdir);
    if !monitor_enabled.load(Ordering::Relaxed) {
        repo_load_trace::trace!("repo_monitor_exit_before_start repo_id={:?}", repo_id);
        return;
    }
    let git_dir = resolve_git_dir(&workdir);
    let mut gitignore = GitignoreRules::load(&workdir, backend);

    // The set of worktree subdirectories currently watched per-directory, kept in sync as the tree
    // changes (deduped on re-watch, pruned on deletion). `build_workdir_watcher` repopulates it; its
    // length is the live-watch count enforced against the budget.
    let mut watched_dirs: FxHashSet<PathBuf> = FxHashSet::default();

    let Some((mut watcher, mut watch_outcome)) = build_workdir_watcher(
        repo_id,
        &workdir,
        git_dir.as_deref(),
        &mut gitignore,
        &mut watched_dirs,
        &monitor_tx,
        &monitor_enabled,
    ) else {
        monitor_enabled.store(false, Ordering::Relaxed);
        return;
    };

    // Warn the user once if live watching of the source tree had to be disabled (too many folders)
    // or could only be partially established; the `.git` watch + focus reload still keep the
    // repository correct.
    let mut watch_degraded = false;
    let mut last_recovery_attempt: Option<Instant> = None;
    note_watch_outcome(&msg_tx, repo_id, &mut watch_degraded, watch_outcome);

    let debounce = Duration::from_millis(250);
    let max_delay = Duration::from_secs(2);
    let idle_tick = Duration::from_secs(30);

    let mut debouncer = DebouncedChange::new(debounce, max_delay);

    let flush = |change: RepoExternalChange| {
        let active = active_repo_id.load(Ordering::Relaxed);
        if active == repo_id.0 {
            trace_repo_monitor_flush("flush", repo_id, change, active);
            msg_tx.send_repo_monitor_or_log(
                Msg::RepoExternallyChanged { repo_id, change },
                "repo monitor flush",
            );
        } else {
            repo_load_trace::trace!(
                "repo_monitor_flush_gated_out source=flush repo_id={:?} active={} change={:?}",
                repo_id,
                active,
                change
            );
        }
    };

    let flush_if_active = |pending: Option<RepoExternalChange>| {
        let Some(change) = pending else {
            return;
        };
        let active = active_repo_id.load(Ordering::Relaxed);
        if active == repo_id.0 {
            trace_repo_monitor_flush("flush_if_active", repo_id, change, active);
            msg_tx.send_repo_monitor_or_log(
                Msg::RepoExternallyChanged { repo_id, change },
                "repo monitor flush_if_active",
            );
        } else {
            repo_load_trace::trace!(
                "repo_monitor_flush_gated_out source=flush_if_active repo_id={:?} active={} change={:?}",
                repo_id,
                active,
                change
            );
        }
    };

    loop {
        let now = Instant::now();
        let timeout = debouncer.next_timeout(now).unwrap_or(idle_tick);

        match monitor_rx.recv_timeout(timeout) {
            Ok(MonitorMsg::Stop) => {
                monitor_enabled.store(false, Ordering::Relaxed);
                break;
            }
            Ok(MonitorMsg::Event(event)) => {
                if !monitor_enabled.load(Ordering::Relaxed) {
                    repo_load_trace::trace!(
                        "repo_monitor_drop_queued_event_after_stop repo_id={:?}",
                        repo_id
                    );
                    break;
                }

                match event {
                    Ok(event) => {
                        // Keep watches in sync with new directories before classifying, so a
                        // freshly-created tree's future contents are observed. Its current contents
                        // are reflected by the refresh this event already triggers. Worktree
                        // subdirs are only added when we are watching the subtree (not in the
                        // "too many folders" mode); the `.git` subtree (rebase/merge state) is
                        // always kept in sync since the `.git` watch is always active.
                        if watch_outcome.watches_subtree() {
                            prune_removed_worktree_dirs(&mut watched_dirs, &event);
                            watch_created_dirs(
                                &mut watcher,
                                &mut watched_dirs,
                                MAX_WORKTREE_WATCH_DIRS,
                                &workdir,
                                git_dir.as_deref(),
                                &mut gitignore,
                                &event,
                            );
                        }
                        if let Some(git_dir) = git_dir.as_deref() {
                            watch_created_git_subdirs(&mut watcher, git_dir, &event);
                        }
                        let classified = classify_repo_event(
                            &workdir,
                            git_dir.as_deref(),
                            &mut gitignore,
                            &event,
                        );
                        repo_load_trace::trace!(
                            "monitor_event repo_id={:?} kind={:?} paths={} first={:?} change={:?}",
                            repo_id,
                            event.kind,
                            event.paths.len(),
                            event.paths.first().map(|path| path.display().to_string()),
                            classified.change
                        );
                        if let Some(change) = classified.change {
                            let now = Instant::now();
                            if let Some(to_flush) = debouncer.push(change, now) {
                                flush(to_flush);
                            }
                        }
                        if classified.gitignore_changed {
                            // The ignore rules changed (and `classify_repo_event` already reloaded
                            // them), so re-initiate the worktree watches from scratch by rebuilding
                            // the watcher. Dropping the old watcher releases all of its inotify
                            // watches, so directories that just became ignored stop being watched
                            // (no more churn) and ones that became un-ignored gain watches — keeping
                            // the watched set minimal. The rebuilt watcher is only swapped in if it
                            // sets up successfully, so a transient failure never leaves us watcherless.
                            if let Some((new_watcher, new_outcome)) = build_workdir_watcher(
                                repo_id,
                                &workdir,
                                git_dir.as_deref(),
                                &mut gitignore,
                                &mut watched_dirs,
                                &monitor_tx,
                                &monitor_enabled,
                            ) {
                                watcher = new_watcher;
                                watch_outcome = new_outcome;
                                note_watch_outcome(
                                    &msg_tx,
                                    repo_id,
                                    &mut watch_degraded,
                                    watch_outcome,
                                );
                            }
                        }
                    }
                    Err(_) => {
                        let now = Instant::now();
                        if let Some(to_flush) = debouncer.push(RepoExternalChange::all(), now) {
                            flush(to_flush);
                        }
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !monitor_enabled.load(Ordering::Relaxed) {
                    break;
                }
                let now = Instant::now();
                flush_if_active(debouncer.take_if_due(now));
                // While watching is disabled because the worktree was over budget, an ignore edit in
                // an unwatched subdirectory (which we cannot observe) may have brought it back under
                // budget. Re-check on the idle tick — but throttled, since each re-check reloads the
                // ignore rules — and resume live watching if so.
                if watch_outcome.watches_subtree() {
                    last_recovery_attempt = None;
                } else if recovery_recheck_due(
                    last_recovery_attempt,
                    now,
                    DEGRADED_WATCH_RECHECK_INTERVAL,
                ) {
                    last_recovery_attempt = Some(now);
                    if let Some((new_watcher, new_outcome)) = attempt_degraded_watch_recovery(
                        watch_outcome,
                        repo_id,
                        &workdir,
                        git_dir.as_deref(),
                        &mut gitignore,
                        &mut watched_dirs,
                        &monitor_tx,
                        &monitor_enabled,
                    ) {
                        watcher = new_watcher;
                        watch_outcome = new_outcome;
                        note_watch_outcome(&msg_tx, repo_id, &mut watch_degraded, watch_outcome);
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                monitor_enabled.store(false, Ordering::Relaxed);
                break;
            }
        }
    }
    monitor_enabled.store(false, Ordering::Relaxed);
}

#[cfg(any(target_os = "linux", test))]
fn is_ignored_dir(workdir: &Path, gitignore: &mut GitignoreRules, path: &Path) -> bool {
    is_ignored_worktree_path_with_hint(workdir, gitignore, path, Some(true))
}

/// Collects every directory in `start`'s subtree that the monitor should watch: it skips the git
/// directory and any gitignored directory (e.g. `target/`), and never follows symlinks. `start`
/// is included when it is itself watchable. Adding a per-directory (non-recursive) watch for only
/// these directories keeps churn under large ignored build dirs out of the event queue entirely,
/// which on Linux is what prevents inotify-queue overflow from dropping real worktree edits.
/// (Production code uses [`collect_watchable_dirs_capped`]; this uncapped convenience is for tests.)
#[cfg(test)]
fn collect_watchable_dirs(
    start: &Path,
    workdir: &Path,
    git_dir: Option<&Path>,
    gitignore: &mut GitignoreRules,
) -> Vec<PathBuf> {
    collect_watchable_dirs_capped(start, workdir, git_dir, gitignore, usize::MAX)
}

/// Like [`collect_watchable_dirs`] but stops early once more than `max_subdirs` non-root directories
/// have been collected, so a worktree far over budget does not pay for a full walk just to learn it
/// is over budget. The returned vec still includes `start`; on overflow it holds `max_subdirs + 2`
/// entries (enough for the caller to see the count exceeded the budget).
#[cfg(any(target_os = "linux", test))]
fn collect_watchable_dirs_capped(
    start: &Path,
    workdir: &Path,
    git_dir: Option<&Path>,
    gitignore: &mut GitignoreRules,
    max_subdirs: usize,
) -> Vec<PathBuf> {
    let mut result = Vec::new();
    if is_git_related_path(workdir, git_dir, start) {
        return result;
    }
    if start != workdir && is_ignored_dir(workdir, gitignore, start) {
        return result;
    }

    let mut stack = vec![start.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir);
        result.push(dir);
        // Stop once we have more subdirectories than the cap (entries beyond `start`); the caller
        // only needs to know the budget was exceeded, not the exact total.
        if result.len() > max_subdirs.saturating_add(1) {
            break;
        }
        let Ok(entries) = entries else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            // Files and symlinks (including symlinked directories) are not descended into.
            if !file_type.is_dir() {
                continue;
            }
            let path = entry.path();
            if is_git_related_path(workdir, git_dir, &path)
                || is_ignored_dir(workdir, gitignore, &path)
            {
                continue;
            }
            stack.push(path);
        }
    }
    result
}

/// Adds non-recursive watches for the directories in `start`'s subtree, respecting the watch budget:
/// it stops once `watched_dirs` reaches `max_dirs` so a large directory moved into the worktree at
/// runtime cannot grow the watch set without bound (which would exhaust the kernel inotify limit and
/// silently drop later watches). Directories already present in `watched_dirs` are skipped (no
/// double counting on re-watch), and newly watched ones are inserted into the set.
#[cfg(target_os = "linux")]
fn add_subtree_watches(
    watcher: &mut RecommendedWatcher,
    watched_dirs: &mut FxHashSet<PathBuf>,
    max_dirs: usize,
    start: &Path,
    workdir: &Path,
    git_dir: Option<&Path>,
    gitignore: &mut GitignoreRules,
) {
    let remaining = max_dirs.saturating_sub(watched_dirs.len());
    if remaining == 0 {
        repo_load_trace::trace!(
            "monitor_runtime_watch_budget_reached watched={} max={}",
            watched_dirs.len(),
            max_dirs
        );
        return;
    }
    for dir in collect_watchable_dirs_capped(start, workdir, git_dir, gitignore, remaining) {
        if watched_dirs.len() >= max_dirs {
            repo_load_trace::trace!(
                "monitor_runtime_watch_budget_reached watched={} max={}",
                watched_dirs.len(),
                max_dirs
            );
            break;
        }
        if watched_dirs.contains(&dir) {
            continue;
        }
        if watcher.watch(&dir, RecursiveMode::NonRecursive).is_ok() {
            watched_dirs.insert(dir);
        }
    }
}

/// Above this many non-ignored worktree directories the monitor stops watching source folders
/// entirely and relies on the `.git` watch plus the focus-triggered full refresh. One inotify watch
/// is needed per directory, and the default `fs.inotify.max_user_watches` is as low as 8192 on many
/// systems (and lower in Flatpak/containers); watching a worktree of thousands of folders would
/// exhaust that limit, stall setup, and risk event-queue overflow. Focus reload re-reads the whole
/// worktree, so correctness is preserved — only live, in-focus edit detection is dropped.
const MAX_WORKTREE_WATCH_DIRS: usize = 4096;

/// Outcome of setting up the worktree watches, so the caller can warn the user when live watching
/// of the source tree was disabled or only partially established.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WatchSetupOutcome {
    /// The workdir root and the non-ignored worktree subdirectories are watched per-directory. The
    /// set of watched subdirectories is tracked separately by the monitor thread (so it can be
    /// deduped on re-watch and pruned on deletion); `failed_dirs` is non-zero when some per-directory
    /// watches could not be added (the kernel inotify limit was reached partway through), i.e. the
    /// worktree is only partially watched.
    Watching { failed_dirs: usize },
    /// Too many non-ignored worktree directories: no source folders are watched (only the workdir
    /// root); the source tree is left to the `.git` watch + focus-triggered full refresh. Carries
    /// the subdirectory count for the user-facing warning.
    #[cfg(any(target_os = "linux", test))]
    WorktreeSubdirsSkipped { dir_count: usize },
    /// The workdir root watch failed; the watcher is unusable.
    RootWatchFailed,
}

impl WatchSetupOutcome {
    /// Whether the worktree subtree is watched per-directory, so newly-created directories need
    /// watches added (and removed directories pruned) at runtime. False in the skipped/failed modes.
    fn watches_subtree(self) -> bool {
        matches!(self, WatchSetupOutcome::Watching { .. })
    }
}

/// Sets up the workdir watches. On Linux the recursive watcher is replaced with per-directory
/// non-recursive watches over only the non-ignored tree, so massive ignored build dirs (`target/`,
/// `.flatpak-builder/`, …) never enqueue events that could overflow the inotify queue and drop
/// real worktree edits. Other platforms keep the single recursive watch (their backends do not
/// share inotify's queue-overflow failure mode).
#[cfg(target_os = "linux")]
fn setup_workdir_watch(
    watcher: &mut RecommendedWatcher,
    watched_dirs: &mut FxHashSet<PathBuf>,
    workdir: &Path,
    git_dir: Option<&Path>,
    gitignore: &mut GitignoreRules,
    repo_id: RepoId,
) -> WatchSetupOutcome {
    setup_workdir_watch_with_limit(
        watcher,
        watched_dirs,
        workdir,
        git_dir,
        gitignore,
        repo_id,
        MAX_WORKTREE_WATCH_DIRS,
    )
}

/// Implementation of the Linux worktree watch setup, with the directory budget injected so tests can
/// exercise the "too many folders" path without creating thousands of directories. `watched_dirs` is
/// cleared and repopulated with the worktree subdirectories that were successfully watched.
#[cfg(target_os = "linux")]
fn setup_workdir_watch_with_limit(
    watcher: &mut RecommendedWatcher,
    watched_dirs: &mut FxHashSet<PathBuf>,
    workdir: &Path,
    git_dir: Option<&Path>,
    gitignore: &mut GitignoreRules,
    repo_id: RepoId,
    max_dirs: usize,
) -> WatchSetupOutcome {
    watched_dirs.clear();
    // Always watch the workdir root non-recursively: it is cheap, catches edits to root-level files,
    // and — crucially — observes root `.gitignore` edits so the watcher can re-initiate (and so a
    // worktree that drops below the budget after an ignore edit can start watching its source tree).
    if let Err(error) = watcher.watch(workdir, RecursiveMode::NonRecursive) {
        record_monitor_failure(
            MonitorFailureKind::Start,
            "repo_monitor_thread watch workdir",
            format!(
                "repo_id={repo_id:?}, workdir={}: {error}",
                workdir.display()
            ),
        );
        return WatchSetupOutcome::RootWatchFailed;
    }

    // Capped walk: a worktree far over budget stops the walk early instead of enumerating every
    // directory just to learn it is over budget.
    let dirs = collect_watchable_dirs_capped(workdir, workdir, git_dir, gitignore, max_dirs);
    let subdir_count = dirs.len().saturating_sub(1);

    if subdir_count > max_dirs {
        // Too many folders to watch within the kernel limit: do not watch any source folders. The
        // `.git` watch keeps git operations live, and focus reload re-reads the whole worktree.
        repo_load_trace::trace!(
            "monitor_setup_watches_skipped repo_id={:?} workdir={} subdirs={} max={}",
            repo_id,
            workdir.display(),
            subdir_count,
            max_dirs
        );
        // This runs on the unguarded monitor thread, and a degraded-watch repo
        // is a path users hit routinely (see process::write_stderr_line).
        gitcomet_core::process::write_stderr_line(format_args!(
            "gitcomet-state: repo monitor is not watching the {subdir_count} worktree folders of \
             repo_id={repo_id:?} (workdir={}) because that exceeds the watch budget ({max_dirs}); \
             live file watching is disabled and changes refresh when the window regains focus. Add \
             build/output dirs to .gitignore or raise fs.inotify.max_user_watches to re-enable.",
            workdir.display(),
        ));
        return WatchSetupOutcome::WorktreeSubdirsSkipped {
            dir_count: subdir_count,
        };
    }

    let mut watched = 0usize;
    let mut failed = 0usize;
    let mut first_failure: Option<String> = None;
    for dir in &dirs {
        if dir == workdir {
            continue;
        }
        match watcher.watch(dir, RecursiveMode::NonRecursive) {
            Ok(()) => {
                watched += 1;
                watched_dirs.insert(dir.clone());
            }
            Err(error) => {
                failed += 1;
                if first_failure.is_none() {
                    first_failure = Some(format!("{}: {error}", dir.display()));
                }
            }
        }
    }
    repo_load_trace::trace!(
        "monitor_setup_watches repo_id={:?} workdir={} subdirs_watched={} subdirs_failed={} total_subdirs={} first_failure={:?}",
        repo_id,
        workdir.display(),
        watched,
        failed,
        subdir_count,
        first_failure
    );
    if failed > 0 {
        // Some per-directory watches could not be added (typically the kernel inotify watch limit).
        // The worktree is then only partially watched, so some external edits will not be observed
        // until the next refresh. Surface it so the limit can be raised if it keeps happening; the
        // `failed_dirs` count also drives the user-facing degraded-watch warning.
        gitcomet_core::process::write_stderr_line(format_args!(
            "gitcomet-state: repo monitor could not watch {failed}/{subdir_count} worktree \
             subdirectories for repo_id={repo_id:?} (workdir={}); some external changes may be \
             missed until the next refresh. If this persists, raise fs.inotify.max_user_watches. \
             first_failure={first_failure:?}",
            workdir.display(),
        ));
    }
    WatchSetupOutcome::Watching {
        failed_dirs: failed,
    }
}

#[cfg(not(target_os = "linux"))]
fn setup_workdir_watch(
    watcher: &mut RecommendedWatcher,
    _watched_dirs: &mut FxHashSet<PathBuf>,
    workdir: &Path,
    _git_dir: Option<&Path>,
    _gitignore: &mut GitignoreRules,
    repo_id: RepoId,
) -> WatchSetupOutcome {
    if let Err(error) = watcher
        .watch(workdir, RecursiveMode::Recursive)
        .or_else(|_| watcher.watch(workdir, RecursiveMode::NonRecursive))
    {
        record_monitor_failure(
            MonitorFailureKind::Start,
            "repo_monitor_thread watch workdir",
            format!(
                "repo_id={repo_id:?}, workdir={}: {error}",
                workdir.display()
            ),
        );
        return WatchSetupOutcome::RootWatchFailed;
    }
    // Other backends use a single recursive watch with no per-directory budget (the set stays empty).
    WatchSetupOutcome::Watching { failed_dirs: 0 }
}

/// Whether an event introduces a new directory whose future contents we may need to watch.
#[cfg(target_os = "linux")]
fn event_brings_in_new_dir(event: &notify::Event) -> bool {
    matches!(
        event.kind,
        notify::EventKind::Create(_)
            | notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
    )
}

/// Adds watches for directories that just appeared (created or moved in) so their future contents
/// are observed; the current contents are already reflected by the refresh the event triggers. The
/// watch budget is honoured via `watched_dirs`/`max_dirs`. No-op off Linux, where the recursive
/// watcher picks up new directories itself.
#[cfg(target_os = "linux")]
fn watch_created_dirs(
    watcher: &mut RecommendedWatcher,
    watched_dirs: &mut FxHashSet<PathBuf>,
    max_dirs: usize,
    workdir: &Path,
    git_dir: Option<&Path>,
    gitignore: &mut GitignoreRules,
    event: &notify::Event,
) {
    if !event_brings_in_new_dir(event) {
        return;
    }
    for path in &event.paths {
        let is_dir = std::fs::symlink_metadata(path)
            .map(|metadata| metadata.is_dir())
            .unwrap_or(false);
        if is_dir {
            add_subtree_watches(
                watcher,
                watched_dirs,
                max_dirs,
                path,
                workdir,
                git_dir,
                gitignore,
            );
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn watch_created_dirs(
    _watcher: &mut RecommendedWatcher,
    _watched_dirs: &mut FxHashSet<PathBuf>,
    _max_dirs: usize,
    _workdir: &Path,
    _git_dir: Option<&Path>,
    _gitignore: &mut GitignoreRules,
    _event: &notify::Event,
) {
}

/// Drops directories that were removed (or renamed away) from the tracked watch set so the live-watch
/// count reflects reality. The kernel auto-removes the inotify watch when a watched directory is
/// deleted, but our budget counter would otherwise keep counting it and prematurely refuse new
/// watches. No-op off Linux (no per-directory set is tracked there).
#[cfg(target_os = "linux")]
fn prune_removed_worktree_dirs(watched_dirs: &mut FxHashSet<PathBuf>, event: &notify::Event) {
    if !matches!(event.kind, notify::EventKind::Remove(_)) {
        return;
    }
    for path in &event.paths {
        watched_dirs.retain(|watched| watched != path && !watched.starts_with(path));
    }
}

#[cfg(not(target_os = "linux"))]
fn prune_removed_worktree_dirs(_watched_dirs: &mut FxHashSet<PathBuf>, _event: &notify::Event) {}

/// Adds a recursive watch for a git-dir subdirectory that appears at runtime — e.g. `rebase-merge/`,
/// `rebase-apply/`, or `sequencer/`, created when an interactive rebase / cherry-pick / revert
/// starts — unless it is denylisted (see [`is_denylisted_git_subdir`]). Without this, the git-dir
/// root watch (non-recursive) would see the directory's creation but not the step-state writes
/// inside it during the operation. No-op off Linux, where the recursive `.git` watcher covers it.
#[cfg(target_os = "linux")]
fn watch_created_git_subdirs(
    watcher: &mut RecommendedWatcher,
    git_dir: &Path,
    event: &notify::Event,
) {
    if !event_brings_in_new_dir(event) {
        return;
    }
    for path in &event.paths {
        let is_new_git_subdir = path.parent() == Some(git_dir)
            && !is_denylisted_git_subdir(path)
            && std::fs::symlink_metadata(path)
                .map(|metadata| metadata.is_dir())
                .unwrap_or(false);
        if is_new_git_subdir {
            let _ = watcher
                .watch(path, RecursiveMode::Recursive)
                .or_else(|_| watcher.watch(path, RecursiveMode::NonRecursive));
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn watch_created_git_subdirs(
    _watcher: &mut RecommendedWatcher,
    _git_dir: &Path,
    _event: &notify::Event,
) {
}

fn trace_repo_monitor_flush(
    source: &'static str,
    repo_id: RepoId,
    change: RepoExternalChange,
    active_repo: u64,
) {
    repo_load_trace::trace!(
        "repo_monitor_flush source={} repo_id={:?} change_worktree={} change_index={} change_git_state={} active_repo={}",
        source,
        repo_id,
        change.worktree,
        change.index,
        change.git_state,
        active_repo
    );
}

fn resolve_git_dir(workdir: &Path) -> Option<PathBuf> {
    let dot_git = workdir.join(".git");
    let md = fs::metadata(&dot_git).ok()?;

    if md.is_dir() {
        return Some(dot_git);
    }

    if !md.is_file() {
        return None;
    }

    let contents = fs::read_to_string(&dot_git).ok()?;
    let line = contents.lines().next()?.trim();
    let gitdir = line.strip_prefix("gitdir:")?.trim();
    if gitdir.is_empty() {
        return None;
    }

    let path = PathBuf::from(gitdir);
    if path.is_absolute() {
        Some(path)
    } else {
        Some(workdir.join(path))
    }
}

fn merge_change(a: RepoExternalChange, b: RepoExternalChange) -> RepoExternalChange {
    RepoExternalChange {
        worktree: a.worktree || b.worktree,
        index: a.index || b.index,
        git_state: a.git_state || b.git_state,
        tags: a.tags || b.tags,
    }
}

/// Result of classifying a watcher event: the (optional) coalesced change to refresh, and whether
/// the ignore configuration changed (so the caller can re-initiate the worktree watches without
/// re-scanning the paths or re-loading the rules — this function already reloaded them in place).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ClassifiedEvent {
    change: Option<RepoExternalChange>,
    gitignore_changed: bool,
}

impl ClassifiedEvent {
    fn none() -> Self {
        Self {
            change: None,
            gitignore_changed: false,
        }
    }
}

fn classify_repo_event(
    workdir: &Path,
    git_dir: Option<&Path>,
    gitignore: &mut GitignoreRules,
    event: &notify::Event,
) -> ClassifiedEvent {
    if should_ignore_event_kind(event) {
        return ClassifiedEvent::none();
    }

    // Detect (and reload on) ignore-config changes once, up front, so the result is reliable even on
    // a rescan event and the caller never needs a second scan or a second `GitignoreRules::load`.
    let gitignore_changed = event
        .paths
        .iter()
        .any(|p| is_gitignore_config_path(workdir, git_dir, p));
    if gitignore_changed {
        gitignore.reload(workdir);
    }

    // If notify indicates a rescan is needed, assume anything could have changed.
    if event.need_rescan() {
        return ClassifiedEvent {
            change: Some(RepoExternalChange::all()),
            gitignore_changed,
        };
    }

    if gitignore_changed {
        return ClassifiedEvent {
            change: Some(RepoExternalChange::worktree()),
            gitignore_changed: true,
        };
    }

    if event.paths.is_empty() {
        return ClassifiedEvent {
            change: Some(RepoExternalChange::all()),
            gitignore_changed: false,
        };
    }

    let mut saw_worktree = false;
    let mut saw_index = false;
    let mut saw_git_state = false;
    let mut saw_tags = false;
    let is_dir_hint = path_dir_hint(event);

    for path in &event.paths {
        if is_git_index_lock_path(workdir, git_dir, path) {
            continue;
        }
        if is_git_related_path(workdir, git_dir, path) {
            if is_git_index_path(workdir, git_dir, path) {
                saw_index = true;
            } else {
                saw_git_state = true;
                if is_git_tags_path(workdir, git_dir, path) {
                    saw_tags = true;
                }
            }
        } else {
            if is_ignored_worktree_path_with_hint(workdir, gitignore, path, is_dir_hint) {
                continue;
            }
            saw_worktree = true;
        }
    }

    let change = RepoExternalChange {
        worktree: saw_worktree,
        index: saw_index,
        git_state: saw_git_state,
        tags: saw_tags,
    };
    ClassifiedEvent {
        change: (!change.is_empty()).then_some(change),
        gitignore_changed: false,
    }
}

fn is_git_related_path(workdir: &Path, git_dir: Option<&Path>, path: &Path) -> bool {
    let dot_git = workdir.join(".git");
    if path == dot_git || path.starts_with(&dot_git) {
        return true;
    }
    git_dir.is_some_and(|git_dir| path.starts_with(git_dir))
}

fn is_git_index_path(workdir: &Path, git_dir: Option<&Path>, path: &Path) -> bool {
    let dot_git = workdir.join(".git");
    if path == dot_git.join("index") {
        return true;
    }

    if let Some(git_dir) = git_dir
        && path == git_dir.join("index")
    {
        return true;
    }

    false
}

fn is_git_index_lock_path(workdir: &Path, git_dir: Option<&Path>, path: &Path) -> bool {
    let dot_git = workdir.join(".git");
    if path == dot_git.join("index.lock") {
        return true;
    }

    if let Some(git_dir) = git_dir
        && path == git_dir.join("index.lock")
    {
        return true;
    }

    false
}

fn is_git_tags_path(workdir: &Path, git_dir: Option<&Path>, path: &Path) -> bool {
    let dot_git = workdir.join(".git");
    let tags_dir = dot_git.join("refs").join("tags");
    let packed_refs = dot_git.join("packed-refs");
    if path.starts_with(&tags_dir) || path == packed_refs {
        return true;
    }
    if let Some(git_dir) = git_dir {
        let tags_dir = git_dir.join("refs").join("tags");
        let packed_refs = git_dir.join("packed-refs");
        if path.starts_with(&tags_dir) || path == packed_refs {
            return true;
        }
    }
    false
}

/// Which event kinds the kernel is asked to deliver.
///
/// `notify::Config::default()` is `EventKindMask::ALL`, which on Linux adds
/// `IN_OPEN` and `IN_CLOSE_NOWRITE` to the inotify mask. `should_ignore_event_kind`
/// discards those — but only after the kernel has queued each one and woken the
/// monitor thread. A repo this app is actively reading (status, diffs, `git blame`)
/// makes its own reads generate them, so they routinely account for well over 99%
/// of all delivered events. Ask for only the kinds `classify_repo_event` can act
/// on; `ACCESS_CLOSE` (`IN_CLOSE_WRITE`) is the one access kind that signals a
/// completed write, and `should_ignore_event_kind` still keeps exactly that one.
const WATCHED_EVENT_KINDS: EventKindMask = EventKindMask::CORE.union(EventKindMask::ACCESS_CLOSE);

fn should_ignore_event_kind(event: &notify::Event) -> bool {
    match &event.kind {
        // Reading repo state should not cause a refresh loop; ignore access events except
        // close-after-write which indicates a write has completed. Backends that
        // honour `WATCHED_EVENT_KINDS` no longer deliver the ignored kinds at all;
        // this stays as the portable backstop.
        notify::EventKind::Access(AccessKind::Close(AccessMode::Write)) => false,
        notify::EventKind::Access(_) => true,
        _ => false,
    }
}

fn is_gitignore_config_path(workdir: &Path, git_dir: Option<&Path>, path: &Path) -> bool {
    if path.starts_with(workdir)
        && path
            .file_name()
            .is_some_and(|name| name == std::ffi::OsStr::new(".gitignore"))
    {
        return true;
    }
    git_dir.is_some_and(|git_dir| path == git_dir.join("info").join("exclude"))
}

fn is_ignored_worktree_path_with_hint(
    workdir: &Path,
    gitignore: &mut GitignoreRules,
    path: &Path,
    is_dir_hint: Option<bool>,
) -> bool {
    let Ok(rel) = path.strip_prefix(workdir) else {
        return false;
    };
    gitignore.is_ignored_rel(rel, is_dir_hint)
}

fn path_dir_hint(event: &notify::Event) -> Option<bool> {
    match &event.kind {
        notify::EventKind::Create(kind) => match kind {
            notify::event::CreateKind::Folder => Some(true),
            notify::event::CreateKind::File => Some(false),
            _ => None,
        },
        notify::EventKind::Remove(kind) => match kind {
            notify::event::RemoveKind::Folder => Some(true),
            notify::event::RemoveKind::File => Some(false),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::EventKind;
    use notify::event::{AccessKind, AccessMode, CreateKind, DataChange, ModifyKind, RemoveKind};
    use std::fs;
    use std::process::Command;
    use std::sync::{OnceLock, atomic::AtomicBool, mpsc};
    struct IsolatedGitConfigEnv {
        _root: tempfile::TempDir,
        home_dir: PathBuf,
        xdg_config_home: PathBuf,
        global_config: PathBuf,
        excludes_file: PathBuf,
    }

    fn isolated_git_config_env() -> &'static IsolatedGitConfigEnv {
        static ENV: OnceLock<IsolatedGitConfigEnv> = OnceLock::new();
        ENV.get_or_init(|| {
            let root = tempfile::tempdir().expect("create isolated git config tempdir");
            let home_dir = root.path().join("home");
            let xdg_config_home = root.path().join("xdg");
            let global_config = root.path().join("global.gitconfig");
            let excludes_file = root.path().join("global-excludes");

            fs::create_dir_all(&home_dir).expect("create isolated HOME directory");
            fs::create_dir_all(&xdg_config_home)
                .expect("create isolated XDG_CONFIG_HOME directory");
            fs::write(&global_config, "").expect("create isolated global git config file");
            fs::write(&excludes_file, "").expect("create isolated excludes file");

            IsolatedGitConfigEnv {
                _root: root,
                home_dir,
                xdg_config_home,
                global_config,
                excludes_file,
            }
        })
    }

    fn run_git(repo: &Path, args: &[&str]) {
        let env = isolated_git_config_env();
        let output = Command::new("git")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", &env.global_config)
            .env("HOME", &env.home_dir)
            .env("XDG_CONFIG_HOME", &env.xdg_config_home)
            .env_remove("GIT_CONFIG_SYSTEM")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "Never")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .expect("run git command");
        assert!(
            output.status.success(),
            "git {:?} failed: stdout={} stderr={}",
            args,
            String::from_utf8(output.stdout).unwrap_or_else(|_| "<non-utf8 stdout>".to_string()),
            String::from_utf8(output.stderr).unwrap_or_else(|_| "<non-utf8 stderr>".to_string())
        );
    }

    fn init_repo_for_ignore_tests(workdir: &Path) {
        let _ = fs::create_dir_all(workdir);
        run_git(workdir, &["init"]);
        // Keep tests deterministic and independent from host global excludes.
        let excludes_file = isolated_git_config_env()
            .excludes_file
            .to_string_lossy()
            .into_owned();
        run_git(workdir, &["config", "core.excludesFile", &excludes_file]);
        run_git(workdir, &["config", "core.fileMode", "false"]);
        run_git(workdir, &["config", "user.email", "you@example.com"]);
        run_git(workdir, &["config", "user.name", "You"]);
        run_git(workdir, &["config", "commit.gpgsign", "false"]);
        // Create an initial commit so that the index file exists (git init
        // doesn't create one until the first staging operation, and the gix
        // excludes stack requires a valid index).
        run_git(workdir, &["commit", "--allow-empty", "-m", "init"]);
    }

    fn load_gitignore_rules(workdir: &Path) -> GitignoreRules {
        GitignoreRules::load(workdir, Arc::new(gitcomet_git_gix::GixBackend))
    }

    fn unique_temp_dir(prefix: &str) -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix(prefix)
            .tempdir()
            .expect("create unique tempdir")
    }

    /// Discard everything a freshly established watch still owes us, returning once it has been
    /// silent for `quiet` (or `budget` runs out).
    ///
    /// Linux may run a file's final `fput` — which is what emits `IN_CLOSE_WRITE` — after `close()`
    /// has already returned to userspace, so a write performed *before* `watch()` can still be
    /// delivered a few hundred microseconds *after* the watch is live. Tests that assert on what a
    /// watch delivers must drain that setup residue first, or they flake under load.
    #[cfg(target_os = "linux")]
    fn drain_until_quiet(
        rx: &mpsc::Receiver<notify::Event>,
        quiet: Duration,
        budget: Duration,
    ) -> Vec<notify::Event> {
        let deadline = Instant::now() + budget;
        let mut drained = Vec::new();
        while Instant::now() < deadline {
            match rx.recv_timeout(quiet) {
                Ok(event) => drained.push(event),
                Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => {
                    break;
                }
            }
        }
        drained
    }

    fn cache_key(rel: impl Into<PathBuf>, is_dir_hint: Option<bool>) -> IgnoreCacheKey {
        IgnoreCacheKey {
            rel: rel.into(),
            is_dir_hint,
        }
    }

    /// Test helper: classify an event and return just the coalesced change (dropping the
    /// `gitignore_changed` signal), matching the pre-`ClassifiedEvent` return shape these tests use.
    fn classify_change(
        workdir: &Path,
        git_dir: Option<&Path>,
        gitignore: &mut GitignoreRules,
        event: &notify::Event,
    ) -> Option<RepoExternalChange> {
        classify_repo_event(workdir, git_dir, gitignore, event).change
    }

    #[test]
    fn resolve_git_dir_handles_dot_git_directory() {
        let dir = unique_temp_dir("gitcomet-monitor-test");
        let workdir = dir.path().join("repo");
        let _ = fs::create_dir_all(workdir.join(".git"));

        assert_eq!(resolve_git_dir(&workdir), Some(workdir.join(".git")));
    }

    #[test]
    fn resolve_git_dir_parses_dot_git_file() {
        let dir = unique_temp_dir("gitcomet-monitor-test");
        let workdir = dir.path().join("repo");
        let gitdir = dir.path().join("actual-git-dir");
        let _ = fs::create_dir_all(&workdir);
        let _ = fs::create_dir_all(&gitdir);

        fs::write(
            workdir.join(".git"),
            format!("gitdir: {}\n", gitdir.display()),
        )
        .expect("write .git file");

        assert_eq!(resolve_git_dir(&workdir), Some(gitdir));
    }

    #[test]
    fn merge_change_coalesces_to_both() {
        assert_eq!(
            merge_change(RepoExternalChange::Worktree, RepoExternalChange::GitState),
            RepoExternalChange {
                worktree: true,
                index: false,
                git_state: true,
                tags: false,
            }
        );
        assert_eq!(
            merge_change(RepoExternalChange::GitState, RepoExternalChange::Worktree),
            RepoExternalChange {
                worktree: true,
                index: false,
                git_state: true,
                tags: false,
            }
        );
        assert_eq!(
            merge_change(RepoExternalChange::Both, RepoExternalChange::Worktree),
            RepoExternalChange::Both
        );
        assert_eq!(
            merge_change(RepoExternalChange::GitState, RepoExternalChange::GitState),
            RepoExternalChange::GitState
        );
    }

    #[test]
    fn classify_repo_change_distinguishes_gitdir_from_worktree() {
        let dir = unique_temp_dir("gitcomet-monitor-test");
        let workdir = dir.path().join("repo");
        let _ = fs::create_dir_all(workdir.join(".git"));

        let event = notify::Event {
            kind: EventKind::Any,
            paths: vec![workdir.join(".git").join("index")],
            attrs: Default::default(),
        };
        assert_eq!(
            classify_change(
                &workdir,
                Some(&workdir.join(".git")),
                &mut GitignoreRules::default(),
                &event
            ),
            Some(RepoExternalChange::Index)
        );

        let event = notify::Event {
            kind: EventKind::Any,
            paths: vec![workdir.join("file.txt")],
            attrs: Default::default(),
        };
        assert_eq!(
            classify_change(
                &workdir,
                Some(&workdir.join(".git")),
                &mut GitignoreRules::default(),
                &event
            ),
            Some(RepoExternalChange::Worktree)
        );

        let event = notify::Event {
            kind: EventKind::Any,
            paths: vec![workdir.join(".git").join("HEAD"), workdir.join("file.txt")],
            attrs: Default::default(),
        };
        assert_eq!(
            classify_change(
                &workdir,
                Some(&workdir.join(".git")),
                &mut GitignoreRules::default(),
                &event
            ),
            Some(RepoExternalChange {
                worktree: true,
                index: false,
                git_state: true,
                tags: false,
            })
        );
    }

    #[test]
    fn classify_repo_change_ignores_git_index_lock_churn() {
        let dir = unique_temp_dir("gitcomet-monitor-test");
        let workdir = dir.path().join("repo");
        let _ = fs::create_dir_all(workdir.join(".git"));

        let mut rules = GitignoreRules::default();
        let create_lock = notify::Event {
            kind: EventKind::Create(CreateKind::File),
            paths: vec![workdir.join(".git").join("index.lock")],
            attrs: Default::default(),
        };
        assert_eq!(
            classify_change(
                &workdir,
                Some(&workdir.join(".git")),
                &mut rules,
                &create_lock
            ),
            None,
            "index.lock creation should not trigger external refresh"
        );

        let mut rules = GitignoreRules::default();
        let remove_lock = notify::Event {
            kind: EventKind::Remove(RemoveKind::File),
            paths: vec![workdir.join(".git").join("index.lock")],
            attrs: Default::default(),
        };
        assert_eq!(
            classify_change(
                &workdir,
                Some(&workdir.join(".git")),
                &mut rules,
                &remove_lock
            ),
            None,
            "index.lock deletion should not trigger external refresh"
        );
    }

    #[test]
    fn classify_repo_change_ignoring_index_lock_does_not_drop_real_worktree_events() {
        let dir = unique_temp_dir("gitcomet-monitor-test");
        let workdir = dir.path().join("repo");
        let _ = fs::create_dir_all(workdir.join(".git"));

        let mut rules = GitignoreRules::default();
        let event = notify::Event {
            kind: EventKind::Create(CreateKind::Any),
            paths: vec![
                workdir.join(".git").join("index.lock"),
                workdir.join("file.txt"),
            ],
            attrs: Default::default(),
        };
        assert_eq!(
            classify_change(&workdir, Some(&workdir.join(".git")), &mut rules, &event),
            Some(RepoExternalChange::Worktree),
            "ignoring index.lock should still classify real worktree changes"
        );
    }

    #[test]
    fn debouncer_flushes_on_debounce_or_max_delay() {
        let base = Instant::now();
        let mut d = DebouncedChange::new(Duration::from_millis(100), Duration::from_millis(250));

        assert_eq!(d.push(RepoExternalChange::Worktree, base), None);
        assert!(d.is_pending());

        // Another event resets debounce window.
        assert_eq!(
            d.push(
                RepoExternalChange::Worktree,
                base + Duration::from_millis(50)
            ),
            None
        );
        assert!(d.next_timeout(base + Duration::from_millis(50)).is_some());

        // Not yet due at 149ms from base.
        assert_eq!(d.take_if_due(base + Duration::from_millis(149)), None);

        // Due by debounce at 150ms from base (last at 50ms + 100ms).
        assert_eq!(
            d.take_if_due(base + Duration::from_millis(150)),
            Some(RepoExternalChange::Worktree)
        );
        assert!(!d.is_pending());

        // Continuous events should flush by max_delay.
        assert_eq!(d.push(RepoExternalChange::GitState, base), None);
        assert_eq!(
            d.push(
                RepoExternalChange::GitState,
                base + Duration::from_millis(300)
            ),
            Some(RepoExternalChange::GitState)
        );
        assert!(!d.is_pending());
    }

    #[test]
    fn access_events_do_not_trigger_refresh_loops() {
        let dir = unique_temp_dir("gitcomet-monitor-test");
        let workdir = dir.path().join("repo");
        let _ = fs::create_dir_all(workdir.join(".git"));

        let event = notify::Event {
            kind: EventKind::Access(AccessKind::Open(AccessMode::Read)),
            paths: vec![workdir.join(".git").join("index")],
            attrs: Default::default(),
        };
        assert_eq!(
            classify_change(
                &workdir,
                Some(&workdir.join(".git")),
                &mut GitignoreRules::default(),
                &event
            ),
            None
        );

        let event = notify::Event {
            kind: EventKind::Access(AccessKind::Close(AccessMode::Read)),
            paths: vec![workdir.join("file.txt")],
            attrs: Default::default(),
        };
        assert_eq!(
            classify_change(
                &workdir,
                Some(&workdir.join(".git")),
                &mut GitignoreRules::default(),
                &event
            ),
            None
        );

        let event = notify::Event {
            kind: EventKind::Access(AccessKind::Close(AccessMode::Write)),
            paths: vec![workdir.join("file.txt")],
            attrs: Default::default(),
        };
        assert_eq!(
            classify_change(
                &workdir,
                Some(&workdir.join(".git")),
                &mut GitignoreRules::default(),
                &event
            ),
            Some(RepoExternalChange::Worktree)
        );
    }

    #[test]
    fn gitignore_rules_match_git_semantics_for_nested_negation_and_anchoring() {
        let dir = unique_temp_dir("gitcomet-monitor-test");
        let workdir = dir.path().join("repo");
        init_repo_for_ignore_tests(&workdir);
        let git_dir = resolve_git_dir(&workdir);

        fs::write(
            workdir.join(".gitignore"),
            "target/\n*.gitcomet-log\n!keep.gitcomet-log\n/build/output\nlogs/*.tmp\n",
        )
        .expect("write .gitignore");
        fs::create_dir_all(workdir.join("logs")).expect("create logs directory");
        fs::write(workdir.join("logs/.gitignore"), "!keep.tmp\n").expect("write nested .gitignore");
        fs::write(
            git_dir
                .as_ref()
                .expect("git dir")
                .join("info")
                .join("exclude"),
            "info-excluded.gitcomet\n",
        )
        .expect("write .git/info/exclude");
        fs::create_dir_all(workdir.join("target/debug")).expect("create target/debug directory");
        // The gix excludes stack traverses directories on disk when processing
        // path components; intermediate dirs must exist (in production, filesystem
        // events always reference existing paths).
        fs::create_dir_all(workdir.join("build")).expect("create build directory");

        let mut rules = load_gitignore_rules(&workdir);
        assert!(rules.is_ignored_rel(Path::new("target/debug/app"), Some(false)));
        assert!(rules.is_ignored_rel(Path::new("foo.gitcomet-log"), Some(false)));
        assert!(!rules.is_ignored_rel(Path::new("keep.gitcomet-log"), Some(false)));
        assert!(rules.is_ignored_rel(Path::new("build/output"), Some(false)));
        assert!(!rules.is_ignored_rel(Path::new("nested/build/output"), Some(false)));
        assert!(rules.is_ignored_rel(Path::new("logs/drop.tmp"), Some(false)));
        assert!(!rules.is_ignored_rel(Path::new("logs/keep.tmp"), Some(false)));
        assert!(rules.is_ignored_rel(Path::new("info-excluded.gitcomet"), Some(false)));
        assert!(rules.is_ignored_rel(Path::new("target"), Some(true)));

        // Ensure folder create events for ignored directories are treated as ignorable worktree
        // changes.
        let event = notify::Event {
            kind: EventKind::Create(CreateKind::Folder),
            paths: vec![workdir.join("target")],
            attrs: Default::default(),
        };
        assert_eq!(
            classify_change(&workdir, git_dir.as_deref(), &mut rules, &event),
            None
        );
    }

    #[test]
    fn tracked_paths_are_not_treated_as_ignored() {
        let dir = unique_temp_dir("gitcomet-monitor-test");
        let workdir = dir.path().join("repo");
        init_repo_for_ignore_tests(&workdir);
        let git_dir = resolve_git_dir(&workdir);

        fs::write(
            workdir.join(".gitignore"),
            "*.tracked-ignore\n*.untracked-ignore\n",
        )
        .expect("write .gitignore");
        fs::write(workdir.join("tracked.tracked-ignore"), "tracked\n").expect("write tracked file");
        fs::write(workdir.join("new.untracked-ignore"), "untracked\n").expect("write ignored file");

        run_git(&workdir, &["add", "-f", "tracked.tracked-ignore"]);

        let mut rules = load_gitignore_rules(&workdir);
        assert!(
            !rules.is_ignored_rel(Path::new("tracked.tracked-ignore"), Some(false)),
            "tracked paths must not be treated as ignored"
        );
        assert!(rules.is_ignored_rel(Path::new("new.untracked-ignore"), Some(false)));

        let tracked_event = notify::Event {
            kind: EventKind::Any,
            paths: vec![workdir.join("tracked.tracked-ignore")],
            attrs: Default::default(),
        };
        assert_eq!(
            classify_change(&workdir, git_dir.as_deref(), &mut rules, &tracked_event),
            Some(RepoExternalChange::Worktree)
        );

        let ignored_event = notify::Event {
            kind: EventKind::Any,
            paths: vec![workdir.join("new.untracked-ignore")],
            attrs: Default::default(),
        };
        assert_eq!(
            classify_change(&workdir, git_dir.as_deref(), &mut rules, &ignored_event),
            None
        );
    }

    #[test]
    fn collect_watchable_dirs_skips_git_and_ignored_directories() {
        let dir = unique_temp_dir("gitcomet-monitor-test");
        let workdir = dir.path().join("repo");
        init_repo_for_ignore_tests(&workdir);
        fs::write(workdir.join(".gitignore"), "target/\n").expect("write .gitignore");
        fs::create_dir_all(workdir.join("target").join("debug")).expect("create target/debug");
        fs::create_dir_all(workdir.join("src").join("sub")).expect("create src/sub");
        let git_dir = resolve_git_dir(&workdir);
        let mut gitignore = load_gitignore_rules(&workdir);

        let dirs = collect_watchable_dirs(&workdir, &workdir, git_dir.as_deref(), &mut gitignore);

        assert!(dirs.contains(&workdir), "workdir root must be watched");
        assert!(
            dirs.contains(&workdir.join("src")),
            "tracked source dir must be watched"
        );
        assert!(
            dirs.contains(&workdir.join("src").join("sub")),
            "nested source dir must be watched"
        );
        assert!(
            !dirs
                .iter()
                .any(|watched| watched.starts_with(workdir.join("target"))),
            "the gitignored build dir must never be watched (this is what avoids the event flood)"
        );
        assert!(
            !dirs
                .iter()
                .any(|watched| watched.starts_with(workdir.join(".git"))),
            "the git dir gets its own recursive watch and must be excluded from the worktree walk"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn setup_workdir_watch_delivers_tracked_events_and_skips_ignored_dirs() {
        // End-to-end check against a real notify watcher: a modification under a tracked directory
        // must be delivered, while churn under the gitignored `target/` must not be watched at all
        // (that is what keeps a build's event flood from drowning real edits).
        let dir = unique_temp_dir("gitcomet-monitor-watch");
        let workdir = dir.path().join("repo");
        init_repo_for_ignore_tests(&workdir);
        fs::write(workdir.join(".gitignore"), "target/\n").expect("write .gitignore");
        fs::create_dir_all(workdir.join("src")).expect("create src");
        fs::create_dir_all(workdir.join("target")).expect("create target");
        let git_dir = resolve_git_dir(&workdir);
        let mut gitignore = load_gitignore_rules(&workdir);

        let (tx, rx) = mpsc::channel::<notify::Event>();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                let _ = tx.send(event);
            }
        })
        .expect("create watcher");
        assert!(
            matches!(
                setup_workdir_watch(
                    &mut watcher,
                    &mut FxHashSet::default(),
                    &workdir,
                    git_dir.as_deref(),
                    &mut gitignore,
                    RepoId(1),
                ),
                WatchSetupOutcome::Watching { failed_dirs: 0 }
            ),
            "a small worktree should be fully watched with no failures"
        );

        // Write under the ignored dir (must be invisible) and under the tracked dir (must arrive).
        fs::write(workdir.join("target").join("artifact.bin"), b"x").expect("write target file");
        fs::write(workdir.join("src").join("main.rs"), b"fn main() {}").expect("write src file");

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut saw_src = false;
        let mut saw_target = false;
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(event) => {
                    for path in &event.paths {
                        saw_src |= path.starts_with(workdir.join("src"));
                        saw_target |= path.starts_with(workdir.join("target"));
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) if saw_src => break,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        assert!(
            saw_src,
            "a modification under a tracked directory must be delivered by the watcher"
        );
        assert!(
            !saw_target,
            "modifications under the gitignored target/ must not be watched"
        );
    }

    #[test]
    fn watched_event_kinds_exclude_read_access_but_keep_close_write() {
        assert!(!WATCHED_EVENT_KINDS.intersects(EventKindMask::ACCESS_OPEN));
        assert!(!WATCHED_EVENT_KINDS.intersects(EventKindMask::ACCESS_CLOSE_NOWRITE));
        // `should_ignore_event_kind` keeps close-after-write, so it must still be requested.
        assert!(WATCHED_EVENT_KINDS.contains(EventKindMask::ACCESS_CLOSE));
        // Everything `classify_repo_event` acts on.
        assert!(WATCHED_EVENT_KINDS.contains(EventKindMask::CORE));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn watcher_does_not_deliver_events_for_reads_of_watched_files() {
        // Regression: the default notify config asks inotify for IN_OPEN and
        // IN_CLOSE_NOWRITE, so every file this app reads while producing a status,
        // diff or blame bounced straight back as an event the monitor then threw
        // away. On an active repo that was >99% of all delivered events — tens of
        // thousands per minute of pure thread-wakeup and allocation churn.
        let dir = unique_temp_dir("gitcomet-monitor-read-noise");
        let workdir = dir.path().join("repo");
        fs::create_dir_all(&workdir).expect("create workdir");
        let file = workdir.join("tracked.txt");
        fs::write(&file, b"before").expect("seed file");

        let (tx, rx) = mpsc::channel::<notify::Event>();
        let mut watcher = RecommendedWatcher::new(
            move |res: notify::Result<notify::Event>| {
                if let Ok(event) = res {
                    let _ = tx.send(event);
                }
            },
            NotifyConfig::default().with_event_kinds(WATCHED_EVENT_KINDS),
        )
        .expect("create watcher");
        watcher
            .watch(&workdir, RecursiveMode::NonRecursive)
            .expect("watch workdir");
        // The seed write closed its file before this watch existed, but its `IN_CLOSE_WRITE` can
        // still land here (see `drain_until_quiet`). Start the read phase from a quiet watch so the
        // assertion below only ever sees events the reads themselves caused.
        let setup_residue =
            drain_until_quiet(&rx, Duration::from_millis(300), Duration::from_secs(5));
        assert!(
            setup_residue
                .iter()
                .all(|event| event.paths == vec![file.clone()]
                    && event.kind
                        == notify::EventKind::Access(AccessKind::Close(AccessMode::Write))),
            "only the seed write may show up before the read phase, got {setup_residue:?}"
        );

        for _ in 0..50 {
            assert_eq!(fs::read(&file).expect("read file"), b"before");
        }
        std::thread::sleep(Duration::from_secs(1));
        let read_events: Vec<_> = rx.try_iter().collect();
        assert!(
            read_events.is_empty(),
            "reading watched files must not deliver any event, got {read_events:?}"
        );

        // The watch is still live: a real write must still arrive.
        fs::write(&file, b"after").expect("modify file");
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut saw_write = false;
        while !saw_write && Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(event) => saw_write = event.paths.iter().any(|p| p == &file),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        assert!(
            saw_write,
            "a write to a watched file must still be delivered"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn ignore_aware_watch_suppresses_build_churn_that_used_to_drown_real_edits() {
        // Regression test for the freshness bug this change fixes: a plain recursive watch over the
        // workdir also watched the gitignored build dir, so a `cargo build`'s churn flooded the
        // event queue (overflowing inotify and dropping/delaying real worktree edits). This test
        // shows the SAME churn is a flood to a recursive (old) watch but produces ZERO events for
        // the ignore-aware (new) setup, while a real edit under a tracked dir is still delivered by
        // both.
        let dir = unique_temp_dir("gitcomet-monitor-churn");
        let workdir = dir.path().join("repo");
        init_repo_for_ignore_tests(&workdir);
        fs::write(workdir.join(".gitignore"), "target/\n").expect("write .gitignore");
        fs::create_dir_all(workdir.join("src")).expect("create src");
        fs::create_dir_all(workdir.join("target")).expect("create target");
        let git_dir = resolve_git_dir(&workdir);
        let mut gitignore = load_gitignore_rules(&workdir);

        let make_watcher = || {
            let (tx, rx) = mpsc::channel::<notify::Event>();
            let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                if let Ok(event) = res {
                    let _ = tx.send(event);
                }
            })
            .expect("create watcher");
            (watcher, rx)
        };

        // Baseline: a naive recursive watch (the old behavior).
        let (mut recursive_watcher, recursive_rx) = make_watcher();
        recursive_watcher
            .watch(&workdir, RecursiveMode::Recursive)
            .expect("recursive watch");

        // The ignore-aware setup under test.
        let (mut ignore_watcher, ignore_rx) = make_watcher();
        assert!(
            matches!(
                setup_workdir_watch(
                    &mut ignore_watcher,
                    &mut FxHashSet::default(),
                    &workdir,
                    git_dir.as_deref(),
                    &mut gitignore,
                    RepoId(1),
                ),
                WatchSetupOutcome::Watching { failed_dirs: 0 }
            ),
            "a small worktree should be fully watched with no failures"
        );

        // Simulate a build: churn a freshly-created subtree under the ignored target/ dir...
        let deps = workdir.join("target").join("debug").join("deps");
        fs::create_dir_all(&deps).expect("create target/debug/deps");
        for i in 0..200 {
            fs::write(deps.join(format!("artifact-{i}.o")), b"obj").expect("write artifact");
        }
        // ...then make a single real edit to a tracked file.
        fs::write(workdir.join("src").join("main.rs"), b"fn main() {}").expect("write src file");

        // Let the OS deliver and buffer every event, then drain each channel without blocking.
        std::thread::sleep(Duration::from_secs(1));
        let drain = |rx: &mpsc::Receiver<notify::Event>| {
            let mut target_events = 0usize;
            let mut saw_src = false;
            while let Ok(event) = rx.try_recv() {
                for path in &event.paths {
                    if path.starts_with(workdir.join("target")) {
                        target_events += 1;
                    }
                    saw_src |= path.starts_with(workdir.join("src"));
                }
            }
            (target_events, saw_src)
        };
        let (recursive_target_events, recursive_saw_src) = drain(&recursive_rx);
        let (ignore_target_events, ignore_saw_src) = drain(&ignore_rx);

        // The old recursive watch is flooded by the ignored build churn (the root cause)...
        assert!(
            recursive_target_events > 0,
            "the naive recursive watch should observe the ignored build churn (it is the flood the \
             old monitor had to process)"
        );
        // ...while the ignore-aware watch never sees a single event from it.
        assert_eq!(
            ignore_target_events, 0,
            "ignore-aware watching must produce no events for gitignored build churn, so it cannot \
             overflow the queue and drop real edits"
        );
        // Both still deliver the real edit under the tracked directory.
        assert!(
            recursive_saw_src && ignore_saw_src,
            "the real edit under a tracked dir must still be delivered (recursive={recursive_saw_src}, \
             ignore_aware={ignore_saw_src})"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn gitignore_change_reinit_unwatches_newly_ignored_dir() {
        // Regression test: when a directory becomes gitignored at runtime, the monitor must
        // re-initiate the worktree watches so the now-ignored tree is no longer watched. Before the
        // fix, the re-watch was add-only and left the stale watches in place, so churn under the
        // freshly-ignored dir kept flooding the event queue (the failure mode behind large worktrees
        // dropping real edits). `build_workdir_watcher` rebuilds the minimal watch set from the
        // current rules, and dropping the previous watcher releases its inotify watches.
        let dir = unique_temp_dir("gitcomet-monitor-reinit");
        let workdir = dir.path().join("repo");
        init_repo_for_ignore_tests(&workdir);
        fs::create_dir_all(workdir.join("vendor").join("pkg")).expect("create vendor/pkg");
        fs::create_dir_all(workdir.join("src")).expect("create src");
        let git_dir = resolve_git_dir(&workdir);
        let mut gitignore = load_gitignore_rules(&workdir);

        let (monitor_tx, monitor_rx) = mpsc::channel::<MonitorMsg>();
        let monitor_enabled = Arc::new(AtomicBool::new(true));
        let mut watched_dirs: FxHashSet<PathBuf> = FxHashSet::default();

        // vendor/ is not yet ignored, so the initial setup watches it.
        let (_initial, _) = build_workdir_watcher(
            RepoId(1),
            &workdir,
            git_dir.as_deref(),
            &mut gitignore,
            &mut watched_dirs,
            &monitor_tx,
            &monitor_enabled,
        )
        .expect("initial watcher build must succeed");
        assert!(
            watched_dirs.contains(&workdir.join("vendor")),
            "vendor/ must be watched before it is ignored"
        );

        // vendor/ becomes gitignored; re-initiate the worktree watches exactly like the monitor
        // loop does: reload the rules and rebuild the watcher. Dropping `_initial` (via the rebind
        // below) releases its watches.
        fs::write(workdir.join(".gitignore"), "vendor/\n").expect("write .gitignore");
        gitignore = load_gitignore_rules(&workdir);
        let (_watcher, _) = build_workdir_watcher(
            RepoId(1),
            &workdir,
            git_dir.as_deref(),
            &mut gitignore,
            &mut watched_dirs,
            &monitor_tx,
            &monitor_enabled,
        )
        .expect("rebuilt watcher must succeed");
        drop(_initial);
        assert!(
            !watched_dirs.contains(&workdir.join("vendor")),
            "rebuilding after the ignore edit must drop vendor/ from the watched set"
        );

        std::thread::sleep(Duration::from_millis(300));
        while monitor_rx.try_recv().is_ok() {}

        // Churn under the now-ignored vendor/ and make one real edit under the tracked src/.
        for i in 0..20 {
            fs::write(
                workdir.join("vendor").join("pkg").join(format!("f{i}.bin")),
                b"x",
            )
            .expect("write vendor churn");
        }
        fs::write(workdir.join("src").join("main.rs"), b"fn main() {}").expect("write src file");

        std::thread::sleep(Duration::from_secs(1));
        let mut vendor_events = 0usize;
        let mut saw_src = false;
        while let Ok(msg) = monitor_rx.try_recv() {
            if let MonitorMsg::Event(Ok(event)) = msg {
                for path in &event.paths {
                    if path.starts_with(workdir.join("vendor")) {
                        vendor_events += 1;
                    }
                    saw_src |= path.starts_with(workdir.join("src"));
                }
            }
        }

        assert!(
            saw_src,
            "a real edit under the tracked src/ must still be delivered"
        );
        assert_eq!(
            vendor_events, 0,
            "vendor/ is now gitignored; re-initiating the watches must unwatch it so its churn \
             produces no events (got {vendor_events})"
        );
    }

    #[test]
    fn watch_degraded_transition_fires_once_per_degraded_episode() {
        let mut degraded = false;
        // Entering the skipped state warns, carrying the folder count.
        assert_eq!(
            watch_degraded_transition(
                &mut degraded,
                WatchSetupOutcome::WorktreeSubdirsSkipped { dir_count: 9000 }
            ),
            Some(RepoWatchDegradedReason::TooManyFolders { dir_count: 9000 })
        );
        // Staying degraded (e.g. a .gitignore rebuild that is still over budget) does not re-warn.
        assert_eq!(
            watch_degraded_transition(
                &mut degraded,
                WatchSetupOutcome::WorktreeSubdirsSkipped { dir_count: 9001 }
            ),
            None
        );
        // Recovering to full watching clears the flag without warning.
        assert_eq!(
            watch_degraded_transition(
                &mut degraded,
                WatchSetupOutcome::Watching { failed_dirs: 0 }
            ),
            None
        );
        assert!(!degraded);
        // A partial watch failure is also a degraded transition and warns with the unwatched count.
        assert_eq!(
            watch_degraded_transition(
                &mut degraded,
                WatchSetupOutcome::Watching { failed_dirs: 7 }
            ),
            Some(RepoWatchDegradedReason::WatchLimitReached { unwatched_dirs: 7 })
        );
        // Still partially failing on a rebuild does not re-warn.
        assert_eq!(
            watch_degraded_transition(
                &mut degraded,
                WatchSetupOutcome::Watching { failed_dirs: 3 }
            ),
            None
        );
    }

    #[test]
    fn degraded_watch_recheck_is_throttled() {
        // Reproduces the "reload ignore rules every idle tick (30s) while degraded" cost: the
        // recovery re-check must be rate-limited, not run on every tick. With no prior attempt it is
        // due; immediately after an attempt it is not; only after the interval elapses is it due again.
        let interval = Duration::from_secs(120);
        let base = Instant::now();
        assert!(
            recovery_recheck_due(None, base, interval),
            "first re-check (no prior attempt) must be due"
        );
        assert!(
            !recovery_recheck_due(Some(base), base + Duration::from_secs(30), interval),
            "a re-check 30s after the last attempt must be throttled (not due)"
        );
        assert!(
            recovery_recheck_due(Some(base), base + interval, interval),
            "a re-check after the full interval must be due again"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn watch_budget_count_does_not_inflate_on_rewatch() {
        // Reproduces the budget-counter drift: re-watching a directory that is already watched must
        // not grow the live-watch count, otherwise create/re-create churn inflates the count past
        // the real number of watches and the monitor refuses new watches while still under budget.
        let dir = unique_temp_dir("gitcomet-monitor-rewatch");
        let workdir = dir.path().join("repo");
        init_repo_for_ignore_tests(&workdir);
        fs::create_dir_all(workdir.join("src").join("sub")).expect("create src/sub");
        let git_dir = resolve_git_dir(&workdir);
        let mut gitignore = load_gitignore_rules(&workdir);

        let (_tx, _rx) = mpsc::channel::<notify::Event>();
        let mut watcher =
            notify::recommended_watcher(move |_res: notify::Result<notify::Event>| {})
                .expect("create watcher");

        let mut watched: FxHashSet<PathBuf> = FxHashSet::default();
        add_subtree_watches(
            &mut watcher,
            &mut watched,
            1000,
            &workdir.join("src"),
            &workdir,
            git_dir.as_deref(),
            &mut gitignore,
        );
        let after_first = watched.len();
        assert!(after_first >= 2, "src + src/sub should be watched");
        assert!(watched.contains(&workdir.join("src")));

        add_subtree_watches(
            &mut watcher,
            &mut watched,
            1000,
            &workdir.join("src"),
            &workdir,
            git_dir.as_deref(),
            &mut gitignore,
        );
        assert_eq!(
            watched.len(),
            after_first,
            "re-watching already-watched directories must not inflate the live-watch count"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn prune_removed_worktree_dirs_drops_deleted_subtree() {
        // A deleted directory (and its descendants) must be dropped from the tracked watch set so the
        // budget reflects reality — the kernel auto-removes the inotify watch, but the set would
        // otherwise keep counting it and prematurely refuse new watches.
        let root = PathBuf::from("/repo");
        let mut watched: FxHashSet<PathBuf> = FxHashSet::default();
        watched.insert(root.join("src"));
        watched.insert(root.join("src/sub"));
        watched.insert(root.join("docs"));

        let remove = notify::Event {
            kind: notify::EventKind::Remove(notify::event::RemoveKind::Folder),
            paths: vec![root.join("src")],
            attrs: Default::default(),
        };
        prune_removed_worktree_dirs(&mut watched, &remove);

        assert!(
            !watched.contains(&root.join("src")) && !watched.contains(&root.join("src/sub")),
            "the removed dir and its descendants must be pruned"
        );
        assert!(
            watched.contains(&root.join("docs")),
            "unrelated watched dirs must be kept"
        );

        // A non-remove event must not prune anything.
        let create = notify::Event {
            kind: notify::EventKind::Create(notify::event::CreateKind::Folder),
            paths: vec![root.join("docs")],
            attrs: Default::default(),
        };
        prune_removed_worktree_dirs(&mut watched, &create);
        assert!(watched.contains(&root.join("docs")));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn over_budget_worktree_skips_source_folder_watches() {
        // When the worktree has more non-ignored folders than the budget, the monitor must not watch
        // any source folders (that is what would exhaust the inotify limit on a massive repo). The
        // workdir root stays watched so root-level edits and .gitignore changes are still observed;
        // everything deeper is left to the .git watch + focus reload.
        let dir = unique_temp_dir("gitcomet-monitor-budget");
        let workdir = dir.path().join("repo");
        init_repo_for_ignore_tests(&workdir);
        fs::create_dir_all(workdir.join("src").join("sub")).expect("create src/sub");
        let git_dir = resolve_git_dir(&workdir);
        let mut gitignore = load_gitignore_rules(&workdir);

        let (tx, rx) = mpsc::channel::<notify::Event>();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                let _ = tx.send(event);
            }
        })
        .expect("create watcher");

        // Budget of 0 forces the "too many folders" path: any worktree subdir exceeds it.
        let mut watched_dirs: FxHashSet<PathBuf> = FxHashSet::default();
        let outcome = setup_workdir_watch_with_limit(
            &mut watcher,
            &mut watched_dirs,
            &workdir,
            git_dir.as_deref(),
            &mut gitignore,
            RepoId(1),
            0,
        );
        assert!(
            matches!(outcome, WatchSetupOutcome::WorktreeSubdirsSkipped { .. }),
            "over-budget worktrees must skip source-folder watches, got {outcome:?}"
        );
        assert!(
            watched_dirs.is_empty(),
            "no worktree subdirs should be tracked in the over-budget mode"
        );

        std::thread::sleep(Duration::from_millis(300));
        while rx.try_recv().is_ok() {}

        // Edits under source folders must NOT be delivered (those folders are not watched)...
        fs::write(workdir.join("src").join("sub").join("deep.rs"), b"x").expect("write deep");
        fs::write(workdir.join("src").join("main.rs"), b"y").expect("write src file");
        // ...but a root-level file edit IS delivered (the workdir root is always watched).
        fs::write(workdir.join("root.txt"), b"z").expect("write root file");

        std::thread::sleep(Duration::from_secs(1));
        let mut saw_src = false;
        let mut saw_root = false;
        while let Ok(event) = rx.try_recv() {
            for path in &event.paths {
                saw_src |= path.starts_with(workdir.join("src"));
                saw_root |= path == &workdir.join("root.txt");
            }
        }
        assert!(
            saw_root,
            "root-level edits must still be delivered (the workdir root is always watched)"
        );
        assert!(
            !saw_src,
            "source folders must not be watched when the worktree is over the watch budget"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn git_dir_watch_excludes_objects_but_sees_root_and_refs() {
        // The git-dir watch must cover the git-state-relevant paths (HEAD at the root, refs/) but
        // deliberately skip the high-churn, high-watch-count objects/ tree.
        let dir = unique_temp_dir("gitcomet-monitor-gitdir");
        let workdir = dir.path().join("repo");
        init_repo_for_ignore_tests(&workdir);
        let git_dir = resolve_git_dir(&workdir).expect("git dir resolves");
        fs::create_dir_all(git_dir.join("refs").join("heads")).expect("create refs/heads");
        fs::create_dir_all(git_dir.join("objects").join("ab")).expect("create objects/ab");

        let (tx, rx) = mpsc::channel::<notify::Event>();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                let _ = tx.send(event);
            }
        })
        .expect("create watcher");
        setup_git_dir_watch(&mut watcher, &git_dir, &workdir, RepoId(1));

        std::thread::sleep(Duration::from_millis(300));
        while rx.try_recv().is_ok() {}

        fs::write(git_dir.join("objects").join("ab").join("deadbeef"), b"obj")
            .expect("write loose object");
        fs::write(git_dir.join("HEAD"), b"ref: refs/heads/main\n").expect("write HEAD");
        fs::write(git_dir.join("refs").join("heads").join("main"), b"abc123\n").expect("write ref");

        std::thread::sleep(Duration::from_secs(1));
        let mut saw_objects = false;
        let mut saw_head = false;
        let mut saw_ref = false;
        while let Ok(event) = rx.try_recv() {
            for path in &event.paths {
                saw_objects |= path.starts_with(git_dir.join("objects"));
                saw_head |= path == &git_dir.join("HEAD");
                saw_ref |= path.starts_with(git_dir.join("refs"));
            }
        }
        assert!(
            !saw_objects,
            ".git/objects must not be watched (a write on every git op, no UI-relevant signal)"
        );
        assert!(
            saw_head,
            "changes to .git/HEAD (git dir root) must be delivered"
        );
        assert!(saw_ref, "changes under .git/refs must be delivered");
    }

    #[test]
    fn gitignore_lookup_stats_track_cache_hits_misses_and_matcher_failures() {
        let before = repo_monitor_ignore_lookup_stats();

        let mut rules = GitignoreRules {
            workdir: Some(PathBuf::from("/tmp/nonexistent")),
            ..Default::default()
        };
        // No matcher — lookups default to not-ignored and count as matcher failures.

        assert!(!rules.is_ignored_rel(Path::new("sample.ignored"), Some(false)));
        assert!(!rules.is_ignored_rel(Path::new("sample.ignored"), Some(false)));

        let after = repo_monitor_ignore_lookup_stats();
        assert!(
            after.request_count >= before.request_count.saturating_add(2),
            "one miss and one hit should each count as ignore lookup requests"
        );
        assert!(
            after.cache_misses >= before.cache_misses.saturating_add(1),
            "the first lookup should miss the cache"
        );
        assert!(
            after.cache_hits >= before.cache_hits.saturating_add(1),
            "the second lookup should hit the cache"
        );
        assert!(
            after.fallback_count >= before.fallback_count.saturating_add(1),
            "disabling the matcher should count as matcher failure"
        );
    }

    #[test]
    fn panic_payload_to_string_handles_string_and_unknown_payloads() {
        assert_eq!(
            panic_payload_to_string(&"panic message".to_string()),
            "panic message"
        );
        assert_eq!(panic_payload_to_string(&123usize), "unknown panic payload");
    }

    #[test]
    fn debouncer_covers_no_pending_due_check_and_max_delay_selection() {
        let base = Instant::now();
        let mut d = DebouncedChange::new(Duration::from_millis(500), Duration::from_millis(100));

        assert_eq!(d.take_if_due(base), None);
        assert_eq!(d.push(RepoExternalChange::Worktree, base), None);

        let timeout = d
            .next_timeout(base + Duration::from_millis(90))
            .expect("pending timeout");
        assert!(
            timeout <= Duration::from_millis(10),
            "max-delay path should schedule the earliest timeout; got {timeout:?}"
        );
    }

    #[test]
    fn resolve_git_dir_parses_relative_dot_git_file() {
        let dir = unique_temp_dir("gitcomet-monitor-test");
        let workdir = dir.path().join("repo");
        fs::create_dir_all(&workdir).expect("create workdir");
        fs::write(workdir.join(".git"), "gitdir: .actual-git\n").expect("write .git file");

        assert_eq!(resolve_git_dir(&workdir), Some(workdir.join(".actual-git")));
    }

    #[test]
    fn classify_repo_event_handles_empty_paths_git_state_and_gitignore_config() {
        let dir = unique_temp_dir("gitcomet-monitor-test");
        let workdir = dir.path().join("repo");
        fs::create_dir_all(workdir.join(".git")).expect("create .git dir");
        let git_dir = Some(workdir.join(".git"));

        let mut rules = GitignoreRules::default();
        let empty_paths = notify::Event {
            kind: EventKind::Any,
            paths: vec![],
            attrs: Default::default(),
        };
        assert_eq!(
            classify_change(&workdir, git_dir.as_deref(), &mut rules, &empty_paths),
            Some(RepoExternalChange::Both)
        );

        let git_head = notify::Event {
            kind: EventKind::Any,
            paths: vec![workdir.join(".git").join("HEAD")],
            attrs: Default::default(),
        };
        assert_eq!(
            classify_change(&workdir, git_dir.as_deref(), &mut rules, &git_head),
            Some(RepoExternalChange::GitState)
        );

        let gitignore_changed = notify::Event {
            kind: EventKind::Any,
            paths: vec![workdir.join(".gitignore")],
            attrs: Default::default(),
        };
        assert_eq!(
            classify_change(&workdir, git_dir.as_deref(), &mut rules, &gitignore_changed),
            Some(RepoExternalChange::Worktree)
        );

        let nested_gitignore_changed = notify::Event {
            kind: EventKind::Any,
            paths: vec![workdir.join("nested").join(".gitignore")],
            attrs: Default::default(),
        };
        assert_eq!(
            classify_change(
                &workdir,
                git_dir.as_deref(),
                &mut rules,
                &nested_gitignore_changed
            ),
            Some(RepoExternalChange::Worktree)
        );
    }

    #[test]
    fn gitignore_cache_enforces_max_size() {
        let mut rules = GitignoreRules::default();
        let now = Instant::now();
        let total = GITIGNORE_CACHE_MAX_ENTRIES + 8;
        for idx in 0..total {
            rules.cache_insert(
                cache_key(format!("path-{idx}.tmp"), Some(false)),
                idx % 2 == 0,
                now + Duration::from_millis(idx as u64),
            );
        }

        assert_eq!(rules.cache.len(), GITIGNORE_CACHE_MAX_ENTRIES);
        assert!(
            !rules
                .cache
                .contains_key(&cache_key("path-0.tmp", Some(false))),
            "oldest entries should be evicted first"
        );
        assert!(
            rules
                .cache
                .contains_key(&cache_key(format!("path-{}.tmp", total - 1), Some(false))),
            "newest entry should remain in cache"
        );
    }

    #[test]
    fn helper_predicates_cover_git_dir_index_strip_prefix_and_remove_hints() {
        let dir = unique_temp_dir("gitcomet-monitor-test");
        let workdir = dir.path().join("repo");
        let git_dir = dir.path().join("worktrees").join("repo");
        fs::create_dir_all(workdir.join(".git")).expect("create .git dir");
        fs::create_dir_all(&git_dir).expect("create detached git dir");

        assert!(is_git_index_path(
            &workdir,
            Some(&git_dir),
            &git_dir.join("index")
        ));
        assert!(is_git_index_lock_path(
            &workdir,
            Some(&git_dir),
            &git_dir.join("index.lock")
        ));
        assert!(is_gitignore_config_path(
            &workdir,
            Some(&git_dir),
            &workdir.join(".gitignore")
        ));
        assert!(is_gitignore_config_path(
            &workdir,
            Some(&git_dir),
            &workdir.join("nested").join(".gitignore")
        ));

        let mut rules = GitignoreRules::default();
        assert!(
            !is_ignored_worktree_path_with_hint(
                &workdir,
                &mut rules,
                dir.path().join("outside.txt").as_path(),
                Some(false),
            ),
            "paths outside the workdir should never be treated as ignored"
        );

        let create_file = notify::Event {
            kind: EventKind::Create(CreateKind::File),
            paths: vec![],
            attrs: Default::default(),
        };
        assert_eq!(path_dir_hint(&create_file), Some(false));

        let remove_folder = notify::Event {
            kind: EventKind::Remove(RemoveKind::Folder),
            paths: vec![],
            attrs: Default::default(),
        };
        assert_eq!(path_dir_hint(&remove_folder), Some(true));

        let remove_file = notify::Event {
            kind: EventKind::Remove(RemoveKind::File),
            paths: vec![],
            attrs: Default::default(),
        };
        assert_eq!(path_dir_hint(&remove_file), Some(false));

        let remove_any = notify::Event {
            kind: EventKind::Remove(RemoveKind::Any),
            paths: vec![],
            attrs: Default::default(),
        };
        assert_eq!(path_dir_hint(&remove_any), None);
    }

    #[test]
    fn gitignore_cache_expires_entries_by_ttl() {
        let mut rules = GitignoreRules::default();
        let now = Instant::now();
        let key = cache_key("stale.txt", Some(false));
        rules.cache_insert(key.clone(), true, now);

        assert_eq!(
            rules.cache_get(&key, now + Duration::from_secs(1)),
            Some(true),
            "fresh cache entry should be returned"
        );
        assert_eq!(
            rules.cache_get(&key, now + GITIGNORE_CACHE_TTL + Duration::from_secs(1)),
            None,
            "expired cache entry should miss"
        );
        assert!(
            !rules.cache.contains_key(&key),
            "expired cache entry should be removed"
        );
    }

    #[test]
    fn watcher_callback_send_is_skipped_when_shutdown_gate_is_closed() {
        let (tx, rx) = mpsc::channel::<MonitorMsg>();
        drop(rx);
        let monitor_enabled = AtomicBool::new(false);
        let did_send = send_watcher_event_or_log(
            RepoId(1),
            &tx,
            Ok(notify::Event {
                kind: EventKind::Any,
                paths: vec![],
                attrs: Default::default(),
            }),
            &monitor_enabled,
        );
        assert!(!did_send, "callback gate should suppress watcher sends");
    }

    #[test]
    fn watcher_callback_send_records_failure_when_gate_is_open() {
        let before = super::super::send_diagnostics::send_failure_count(
            super::super::send_diagnostics::SendFailureKind::RepoMonitorMessage,
        );

        let (tx, rx) = mpsc::channel::<MonitorMsg>();
        drop(rx);
        let monitor_enabled = AtomicBool::new(true);

        let did_send = send_watcher_event_or_log(
            RepoId(1),
            &tx,
            Ok(notify::Event {
                kind: EventKind::Any,
                paths: vec![],
                attrs: Default::default(),
            }),
            &monitor_enabled,
        );
        assert!(
            did_send,
            "callback should attempt sends while monitor is active"
        );

        let after = super::super::send_diagnostics::send_failure_count(
            super::super::send_diagnostics::SendFailureKind::RepoMonitorMessage,
        );
        assert!(after > before);
    }

    #[test]
    fn classify_repo_event_detects_tag_file_changes() {
        let dir = unique_temp_dir("gitcomet-monitor-test");
        let workdir = dir.path().join("repo");
        let git_dir = workdir.join(".git");
        let _ = fs::create_dir_all(git_dir.join("refs").join("tags"));

        let mut rules = GitignoreRules::default();

        // Loose tag file → tags: true
        let tag_event = notify::Event {
            kind: EventKind::Create(CreateKind::File),
            paths: vec![git_dir.join("refs").join("tags").join("v1.0.0")],
            attrs: Default::default(),
        };
        let change = classify_change(&workdir, Some(&git_dir), &mut rules, &tag_event);
        assert_eq!(
            change,
            Some(RepoExternalChange {
                git_state: true,
                tags: true,
                ..Default::default()
            }),
            "tag ref file should produce tags: true"
        );

        // packed-refs → tags: true
        let packed_event = notify::Event {
            kind: EventKind::Modify(ModifyKind::Data(DataChange::Any)),
            paths: vec![git_dir.join("packed-refs")],
            attrs: Default::default(),
        };
        let change = classify_change(&workdir, Some(&git_dir), &mut rules, &packed_event);
        assert_eq!(
            change,
            Some(RepoExternalChange {
                git_state: true,
                tags: true,
                ..Default::default()
            }),
            "packed-refs should produce tags: true"
        );

        // Branch ref file → tags: false
        let branch_event = notify::Event {
            kind: EventKind::Modify(ModifyKind::Data(DataChange::Any)),
            paths: vec![git_dir.join("refs").join("heads").join("main")],
            attrs: Default::default(),
        };
        let change = classify_change(&workdir, Some(&git_dir), &mut rules, &branch_event);
        assert_eq!(
            change,
            Some(RepoExternalChange {
                git_state: true,
                tags: false,
                ..Default::default()
            }),
            "branch ref file should produce tags: false"
        );
    }
}
