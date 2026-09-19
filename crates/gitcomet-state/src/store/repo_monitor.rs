use crate::model::RepoId;
use crate::msg::{Msg, RepoExternalChange, RepoWatchDegradedReason};
use gitcomet_core::services::{GitBackend, WorktreeIgnoreMatcher, WorktreePathKind};
#[cfg(all(target_os = "macos", test))]
use notify::Watcher;
#[cfg(any(not(target_os = "macos"), test))]
use notify::event::EventKindMask;
use notify::event::{AccessKind, AccessMode};
#[cfg(not(target_os = "macos"))]
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

mod ignore_rules;
mod monitor;
mod native_watcher;
mod plan;
mod policy;
use ignore_rules::IgnoreRules;
#[cfg(test)]
use monitor::{EventEffect, MAX_WORKTREE_WATCH_DIRS, MonitorState, summarize};
use monitor::{MonitorConfig, WatchSetupOutcome, repo_monitor_thread};
use native_watcher::MonitorWatcher;
#[cfg(all(test, target_os = "linux"))]
use native_watcher::NativeEventBarrier;
use plan::WatchPlan;
use policy::{
    PathClass, PolicyCell, PolicySnapshot, Triage, WatchInputs, normalized, structural_event,
    triage,
};

enum MonitorMsg {
    Event(notify::Result<notify::Event>),
    Revalidate,
    Stop,
    #[cfg(test)]
    Barrier(mpsc::Sender<()>),
    #[cfg(all(test, target_os = "linux"))]
    Drain(mpsc::Sender<()>),
    #[cfg(all(test, target_os = "macos"))]
    FlushNative(mpsc::Sender<bool>),
    #[cfg(all(test, target_os = "macos"))]
    NativeDrained {
        generation: u64,
        ready: mpsc::Sender<bool>,
    },
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

    pub(super) fn revalidate(&self, repo_id: RepoId) {
        if let Some(handle) = self.handles.get(&repo_id) {
            let _ = handle.msg_tx.send(MonitorMsg::Revalidate);
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
                MonitorConfig::default(),
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

fn watch_degraded_reason(outcome: WatchSetupOutcome) -> Option<RepoWatchDegradedReason> {
    match outcome {
        WatchSetupOutcome::PolicyFailed => Some(RepoWatchDegradedReason::IgnorePolicyFailed),
        WatchSetupOutcome::WorktreeSubdirsSkipped { dir_count } => {
            Some(RepoWatchDegradedReason::TooManyFolders { dir_count })
        }
        WatchSetupOutcome::Watching { failed_dirs } if failed_dirs > 0 => {
            Some(RepoWatchDegradedReason::WatchLimitReached {
                unwatched_dirs: failed_dirs,
            })
        }
        WatchSetupOutcome::Watching { .. } => None,
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
        verification_context: a.verification_context || b.verification_context,
    }
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
#[cfg(any(not(target_os = "macos"), test))]
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
mod tests;
