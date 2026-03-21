use crate::model::RepoId;
use crate::msg::{Msg, RepoExternalChange};
use gix::index::entry::Mode as GitIndexMode;
use notify::event::{AccessKind, AccessMode};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use rustc_hash::FxHashMap as HashMap;
use std::any::Any;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use super::send_diagnostics::{SendFailureKind, send_or_log};

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
    eprintln!(
        "gitcomet-state: repo monitor failure ({kind:?}) in {context}: {detail}; total_failures={count}"
    );
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
    tx: &mpsc::Sender<MonitorMsg>,
    event: notify::Result<notify::Event>,
    callback_enabled: &AtomicBool,
) -> bool {
    if !callback_enabled.load(Ordering::Relaxed) {
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

fn panic_payload_to_string(payload: Box<dyn Any + Send + 'static>) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

fn join_monitor_or_log(join: thread::JoinHandle<()>, repo_id: RepoId, context: &'static str) {
    if let Err(error) = join.join() {
        record_monitor_failure(
            MonitorFailureKind::Join,
            context,
            format!(
                "repo_id={repo_id:?}; join failed: {}",
                panic_payload_to_string(error)
            ),
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
    let average_lookup_nanos = if cache_misses == 0 {
        0
    } else {
        total_lookup_nanos / cache_misses
    };

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
pub(super) fn record_stop_send_failure_for_test(repo_id: RepoId, context: &'static str) {
    let (tx, rx) = mpsc::channel::<MonitorMsg>();
    drop(rx);
    send_stop_or_log(&tx, repo_id, context);
}

#[cfg(test)]
pub(super) fn join_monitor_for_test(
    join: thread::JoinHandle<()>,
    repo_id: RepoId,
    context: &'static str,
) {
    join_monitor_or_log(join, repo_id, context);
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
    handles: HashMap<RepoId, RepoMonitorHandle>,
}

impl RepoMonitorManager {
    pub(super) fn new() -> Self {
        Self {
            handles: HashMap::default(),
        }
    }

    pub(super) fn stop_all(&mut self) {
        for (repo_id, handle) in self.handles.drain() {
            send_stop_or_log(&handle.msg_tx, repo_id, "RepoMonitorManager::stop_all");
            join_monitor_or_log(handle.join, repo_id, "RepoMonitorManager::stop_all");
        }
    }

    pub(super) fn stop(&mut self, repo_id: RepoId) {
        let Some(handle) = self.handles.remove(&repo_id) else {
            return;
        };
        send_stop_or_log(&handle.msg_tx, repo_id, "RepoMonitorManager::stop");
        join_monitor_or_log(handle.join, repo_id, "RepoMonitorManager::stop");
    }

    pub(super) fn running_repo_ids(&self) -> Vec<RepoId> {
        self.handles.keys().copied().collect()
    }

    pub(super) fn start(
        &mut self,
        repo_id: RepoId,
        workdir: PathBuf,
        msg_tx: mpsc::Sender<Msg>,
        active_repo_id: Arc<AtomicU64>,
    ) {
        let std::collections::hash_map::Entry::Vacant(entry) = self.handles.entry(repo_id) else {
            return;
        };
        let (monitor_tx, monitor_rx) = mpsc::channel::<MonitorMsg>();
        let monitor_tx_for_notify = monitor_tx.clone();
        let join = thread::spawn(move || {
            repo_monitor_thread(
                repo_id,
                workdir,
                msg_tx,
                monitor_rx,
                monitor_tx_for_notify,
                active_repo_id,
            )
        });
        entry.insert(RepoMonitorHandle {
            msg_tx: monitor_tx,
            join,
        });
    }
}

struct RepoMonitorHandle {
    msg_tx: mpsc::Sender<MonitorMsg>,
    join: thread::JoinHandle<()>,
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

struct GitignoreMatcher {
    repo: gix::Repository,
    index: gix::worktree::Index,
    excludes: gix::worktree::Stack,
}

impl GitignoreMatcher {
    fn load(workdir: &Path) -> Option<Self> {
        let repo = gix::open(workdir).ok()?;
        let worktree = repo.worktree()?;
        let index = worktree.index().ok()?;
        let excludes = repo
            .excludes(
                &index,
                None,
                gix::worktree::stack::state::ignore::Source::WorktreeThenIdMappingIfNotSkipped,
            )
            .ok()?
            .detach();
        Some(Self {
            repo,
            index,
            excludes,
        })
    }

    fn path_is_tracked(&self, rel: &Path, is_dir_hint: Option<bool>) -> bool {
        let rel = gix::path::to_unix_separators_on_windows(gix::path::into_bstr(rel));
        if self.index.entry_by_path(rel.as_ref()).is_some() {
            return true;
        }

        is_dir_hint == Some(true)
            && self
                .index
                .entry_closest_to_directory_or_directory(rel.as_ref())
                .is_some()
    }

    fn is_ignored_rel(&mut self, rel: &Path, is_dir_hint: Option<bool>) -> Option<bool> {
        if self.path_is_tracked(rel, is_dir_hint) {
            return Some(false);
        }

        let mode = match is_dir_hint {
            Some(true) => Some(GitIndexMode::DIR),
            Some(false) => Some(GitIndexMode::FILE),
            None => None,
        };
        let platform = self.excludes.at_path(rel, mode, &self.repo.objects).ok()?;
        Some(platform.is_excluded())
    }
}

#[derive(Default)]
struct GitignoreRules {
    workdir: Option<PathBuf>,
    matcher: Option<GitignoreMatcher>,
    cache: HashMap<IgnoreCacheKey, CachedIgnoreResult>,
    last_prune_at: Option<Instant>,
}

impl GitignoreRules {
    fn load(workdir: &Path) -> Self {
        Self {
            workdir: Some(workdir.to_path_buf()),
            matcher: GitignoreMatcher::load(workdir),
            cache: HashMap::default(),
            last_prune_at: None,
        }
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
        let (ignored, fresh) = match self.cache.get(key) {
            Some(entry) => (entry.ignored, Self::is_cache_entry_fresh(now, entry)),
            None => return None,
        };

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
        let (ignored, matcher_failed) = match self.matcher.as_mut() {
            Some(matcher) => match matcher.is_ignored_rel(rel, is_dir_hint) {
                Some(ignored) => (ignored, false),
                // gix matcher failed — treat as not-ignored (safe: may cause extra
                // refreshes, but never misses real changes).
                None => (false, true),
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

fn repo_monitor_thread(
    repo_id: RepoId,
    workdir: PathBuf,
    msg_tx: mpsc::Sender<Msg>,
    monitor_rx: mpsc::Receiver<MonitorMsg>,
    monitor_tx: mpsc::Sender<MonitorMsg>,
    active_repo_id: Arc<AtomicU64>,
) {
    let workdir = super::canonicalize_path(workdir);
    let git_dir = resolve_git_dir(&workdir);
    let mut gitignore = GitignoreRules::load(&workdir);
    let callback_enabled = Arc::new(AtomicBool::new(true));

    let watcher = notify::recommended_watcher({
        let monitor_tx = monitor_tx.clone();
        let callback_enabled = Arc::clone(&callback_enabled);
        move |res| {
            send_watcher_event_or_log(&monitor_tx, res, callback_enabled.as_ref());
        }
    });

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
            return;
        }
    };

    if let Err(error) = watcher
        .watch(&workdir, RecursiveMode::Recursive)
        .or_else(|_| watcher.watch(&workdir, RecursiveMode::NonRecursive))
    {
        callback_enabled.store(false, Ordering::Relaxed);
        record_monitor_failure(
            MonitorFailureKind::Start,
            "repo_monitor_thread watch workdir",
            format!(
                "repo_id={repo_id:?}, workdir={}: {error}",
                workdir.display()
            ),
        );
        return;
    }

    if let Some(git_dir) = &git_dir
        && let Err(error) = watcher
            .watch(git_dir, RecursiveMode::Recursive)
            .or_else(|_| watcher.watch(git_dir, RecursiveMode::NonRecursive))
    {
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

    let debounce = Duration::from_millis(250);
    let max_delay = Duration::from_secs(2);
    let idle_tick = Duration::from_secs(30);

    let mut debouncer = DebouncedChange::new(debounce, max_delay);

    let flush = |change: RepoExternalChange| {
        if active_repo_id.load(Ordering::Relaxed) == repo_id.0 {
            send_or_log(
                &msg_tx,
                Msg::RepoExternallyChanged { repo_id, change },
                SendFailureKind::RepoMonitorMessage,
                "repo monitor flush",
            );
        }
    };

    let flush_if_active = |pending: Option<RepoExternalChange>| {
        if let Some(change) = pending
            && active_repo_id.load(Ordering::Relaxed) == repo_id.0
        {
            send_or_log(
                &msg_tx,
                Msg::RepoExternallyChanged { repo_id, change },
                SendFailureKind::RepoMonitorMessage,
                "repo monitor flush_if_active",
            );
        }
    };

    loop {
        let now = Instant::now();
        let timeout = debouncer.next_timeout(now).unwrap_or(idle_tick);

        match monitor_rx.recv_timeout(timeout) {
            Ok(MonitorMsg::Stop) => {
                callback_enabled.store(false, Ordering::Relaxed);
                break;
            }
            Ok(MonitorMsg::Event(Ok(event))) => {
                if let Some(change) =
                    classify_repo_event(&workdir, git_dir.as_deref(), &mut gitignore, &event)
                {
                    let now = Instant::now();
                    if let Some(to_flush) = debouncer.push(change, now) {
                        flush(to_flush);
                    }
                }
            }
            Ok(MonitorMsg::Event(Err(_))) => {
                let now = Instant::now();
                if let Some(to_flush) = debouncer.push(RepoExternalChange::Both, now) {
                    flush(to_flush);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let now = Instant::now();
                flush_if_active(debouncer.take_if_due(now));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                callback_enabled.store(false, Ordering::Relaxed);
                break;
            }
        }
    }
    callback_enabled.store(false, Ordering::Relaxed);
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
    use RepoExternalChange::*;
    match (a, b) {
        (Both, _) | (_, Both) => Both,
        (Worktree, GitState) | (GitState, Worktree) => Both,
        (Worktree, Worktree) => Worktree,
        (GitState, GitState) => GitState,
    }
}

fn classify_repo_event(
    workdir: &Path,
    git_dir: Option<&Path>,
    gitignore: &mut GitignoreRules,
    event: &notify::Event,
) -> Option<RepoExternalChange> {
    if should_ignore_event_kind(event) {
        return None;
    }

    // If notify indicates a rescan is needed, assume anything could have changed.
    if event.need_rescan() {
        return Some(RepoExternalChange::Both);
    }

    // Update ignore rules if the ignore config itself changes.
    if event
        .paths
        .iter()
        .any(|p| is_gitignore_config_path(workdir, git_dir, p))
    {
        *gitignore = GitignoreRules::load(workdir);
        return Some(RepoExternalChange::Worktree);
    }

    if event.paths.is_empty() {
        return Some(RepoExternalChange::Both);
    }

    let mut saw_worktree = false;
    let mut saw_git = false;
    let is_dir_hint = path_dir_hint(event);

    for path in &event.paths {
        if is_git_index_lock_path(workdir, git_dir, path) {
            continue;
        }
        if is_git_related_path(workdir, git_dir, path) {
            // Treat `.git/index` updates like worktree changes: they typically reflect staging
            // operations and should not trigger branch list refreshes.
            if is_git_index_path(workdir, git_dir, path) {
                saw_worktree = true;
            } else {
                saw_git = true;
            }
        } else {
            if is_ignored_worktree_path_with_hint(workdir, gitignore, path, is_dir_hint) {
                continue;
            }
            saw_worktree = true;
        }
        if saw_git && saw_worktree {
            return Some(RepoExternalChange::Both);
        }
    }

    if saw_git {
        Some(RepoExternalChange::GitState)
    } else if saw_worktree {
        Some(RepoExternalChange::Worktree)
    } else {
        None
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

fn should_ignore_event_kind(event: &notify::Event) -> bool {
    match &event.kind {
        // Reading repo state should not cause a refresh loop; ignore access events except
        // close-after-write which indicates a write has completed.
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
    use notify::event::{AccessKind, AccessMode, CreateKind, RemoveKind};
    use std::fs;
    use std::process::Command;
    use std::sync::{OnceLock, atomic::AtomicBool, mpsc};
    use std::time::SystemTime;

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

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ))
    }

    fn cache_key(rel: impl Into<PathBuf>, is_dir_hint: Option<bool>) -> IgnoreCacheKey {
        IgnoreCacheKey {
            rel: rel.into(),
            is_dir_hint,
        }
    }

    #[test]
    fn resolve_git_dir_handles_dot_git_directory() {
        let dir = std::env::temp_dir().join(format!(
            "gitcomet-monitor-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let workdir = dir.join("repo");
        let _ = fs::create_dir_all(workdir.join(".git"));

        assert_eq!(resolve_git_dir(&workdir), Some(workdir.join(".git")));
    }

    #[test]
    fn resolve_git_dir_parses_dot_git_file() {
        let dir = std::env::temp_dir().join(format!(
            "gitcomet-monitor-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let workdir = dir.join("repo");
        let gitdir = dir.join("actual-git-dir");
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
            RepoExternalChange::Both
        );
        assert_eq!(
            merge_change(RepoExternalChange::GitState, RepoExternalChange::Worktree),
            RepoExternalChange::Both
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
        let dir = std::env::temp_dir().join(format!(
            "gitcomet-monitor-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let workdir = dir.join("repo");
        let _ = fs::create_dir_all(workdir.join(".git"));

        let event = notify::Event {
            kind: EventKind::Any,
            paths: vec![workdir.join(".git").join("index")],
            attrs: Default::default(),
        };
        assert_eq!(
            classify_repo_event(
                &workdir,
                Some(&workdir.join(".git")),
                &mut GitignoreRules::default(),
                &event
            ),
            Some(RepoExternalChange::Worktree)
        );

        let event = notify::Event {
            kind: EventKind::Any,
            paths: vec![workdir.join("file.txt")],
            attrs: Default::default(),
        };
        assert_eq!(
            classify_repo_event(
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
            classify_repo_event(
                &workdir,
                Some(&workdir.join(".git")),
                &mut GitignoreRules::default(),
                &event
            ),
            Some(RepoExternalChange::Both)
        );
    }

    #[test]
    fn classify_repo_change_ignores_git_index_lock_churn() {
        let dir = unique_temp_dir("gitcomet-monitor-test");
        let workdir = dir.join("repo");
        let _ = fs::create_dir_all(workdir.join(".git"));

        let mut rules = GitignoreRules::default();
        let create_lock = notify::Event {
            kind: EventKind::Create(CreateKind::File),
            paths: vec![workdir.join(".git").join("index.lock")],
            attrs: Default::default(),
        };
        assert_eq!(
            classify_repo_event(
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
            classify_repo_event(
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
        let workdir = dir.join("repo");
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
            classify_repo_event(&workdir, Some(&workdir.join(".git")), &mut rules, &event),
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
        let dir = std::env::temp_dir().join(format!(
            "gitcomet-monitor-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let workdir = dir.join("repo");
        let _ = fs::create_dir_all(workdir.join(".git"));

        let event = notify::Event {
            kind: EventKind::Access(AccessKind::Open(AccessMode::Read)),
            paths: vec![workdir.join(".git").join("index")],
            attrs: Default::default(),
        };
        assert_eq!(
            classify_repo_event(
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
            classify_repo_event(
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
            classify_repo_event(
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
        let dir = std::env::temp_dir().join(format!(
            "gitcomet-monitor-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let workdir = dir.join("repo");
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

        let mut rules = GitignoreRules::load(&workdir);
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
            classify_repo_event(&workdir, git_dir.as_deref(), &mut rules, &event),
            None
        );
    }

    #[test]
    fn tracked_paths_are_not_treated_as_ignored() {
        let dir = std::env::temp_dir().join(format!(
            "gitcomet-monitor-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let workdir = dir.join("repo");
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

        let mut rules = GitignoreRules::load(&workdir);
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
            classify_repo_event(&workdir, git_dir.as_deref(), &mut rules, &tracked_event),
            Some(RepoExternalChange::Worktree)
        );

        let ignored_event = notify::Event {
            kind: EventKind::Any,
            paths: vec![workdir.join("new.untracked-ignore")],
            attrs: Default::default(),
        };
        assert_eq!(
            classify_repo_event(&workdir, git_dir.as_deref(), &mut rules, &ignored_event),
            None
        );
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
            panic_payload_to_string(Box::new("panic message".to_string())),
            "panic message"
        );
        assert_eq!(
            panic_payload_to_string(Box::new(123usize)),
            "unknown panic payload"
        );
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
        let workdir = dir.join("repo");
        fs::create_dir_all(&workdir).expect("create workdir");
        fs::write(workdir.join(".git"), "gitdir: .actual-git\n").expect("write .git file");

        assert_eq!(resolve_git_dir(&workdir), Some(workdir.join(".actual-git")));
    }

    #[test]
    fn classify_repo_event_handles_empty_paths_git_state_and_gitignore_config() {
        let dir = unique_temp_dir("gitcomet-monitor-test");
        let workdir = dir.join("repo");
        fs::create_dir_all(workdir.join(".git")).expect("create .git dir");
        let git_dir = Some(workdir.join(".git"));

        let mut rules = GitignoreRules::default();
        let empty_paths = notify::Event {
            kind: EventKind::Any,
            paths: vec![],
            attrs: Default::default(),
        };
        assert_eq!(
            classify_repo_event(&workdir, git_dir.as_deref(), &mut rules, &empty_paths),
            Some(RepoExternalChange::Both)
        );

        let git_head = notify::Event {
            kind: EventKind::Any,
            paths: vec![workdir.join(".git").join("HEAD")],
            attrs: Default::default(),
        };
        assert_eq!(
            classify_repo_event(&workdir, git_dir.as_deref(), &mut rules, &git_head),
            Some(RepoExternalChange::GitState)
        );

        let gitignore_changed = notify::Event {
            kind: EventKind::Any,
            paths: vec![workdir.join(".gitignore")],
            attrs: Default::default(),
        };
        assert_eq!(
            classify_repo_event(&workdir, git_dir.as_deref(), &mut rules, &gitignore_changed),
            Some(RepoExternalChange::Worktree)
        );

        let nested_gitignore_changed = notify::Event {
            kind: EventKind::Any,
            paths: vec![workdir.join("nested").join(".gitignore")],
            attrs: Default::default(),
        };
        assert_eq!(
            classify_repo_event(
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
        let workdir = dir.join("repo");
        let git_dir = dir.join("worktrees").join("repo");
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
                dir.join("outside.txt").as_path(),
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
        let callback_enabled = AtomicBool::new(false);
        let did_send = send_watcher_event_or_log(
            &tx,
            Ok(notify::Event {
                kind: EventKind::Any,
                paths: vec![],
                attrs: Default::default(),
            }),
            &callback_enabled,
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
        let callback_enabled = AtomicBool::new(true);

        let did_send = send_watcher_event_or_log(
            &tx,
            Ok(notify::Event {
                kind: EventKind::Any,
                paths: vec![],
                attrs: Default::default(),
            }),
            &callback_enabled,
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
}
