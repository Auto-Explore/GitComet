//! Per-repository load coalescing and log-walk sequencing.

use gitcomet_core::domain::{LogCursor, LogScope};
use gitcomet_core::git_execution_log::context::{self, ExecutionContext};
use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};

/// Loads that coalesce repeated requests into one pending refresh.
/// Log walks, linked-checkout scans, and line statistics have separate protocols.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LoadKind {
    HeadBranch,
    UpstreamDivergence,
    Branches,
    Tags,
    Remotes,
    RemoteBranches,
    WorktreeStatus,
    StagedStatus,
    Stashes,
    Reflog,
    RebaseState,
    MergeCommitMessage,
    RemoteTags,
    Worktrees,
    Submodules,
    RefMetadata,
    /// A worktree walk, deliberately outside the primary refresh.
    FileBrowser,
    /// Repository facts that change less often than status.
    LargeFileSupport,
    /// A server round trip, loaded only on demand.
    LfsLocks,
    /// Serialized because each scan rewrites the numbering used by dropunused.
    AnnexUnused,
}

impl LoadKind {
    const fn mask(self) -> u32 {
        1 << self as u32
    }
}

/// The cancellation epoch shared by a repository's ordinary load tasks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct RepoLoadEpoch(pub(crate) u64);

impl RepoLoadEpoch {
    pub(crate) fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

impl std::fmt::Display for RepoLoadEpoch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Identifies one dispatched walk, including when a filter changes away and back.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct LogLoadSeq(pub(crate) u64);

/// Advances on invalidation, even when the set of changed paths is unchanged.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct LineStatsGeneration(pub(crate) u64);

impl std::fmt::Display for LineStatsGeneration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// A full linked-checkout refresh or an update to specific checkouts.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum WorktreeDirtyScope {
    #[default]
    All,
    Paths(Vec<PathBuf>),
}

impl WorktreeDirtyScope {
    pub(crate) fn includes(&self, path: &Path) -> bool {
        match self {
            Self::All => true,
            Self::Paths(paths) => paths.iter().any(|candidate| candidate == path),
        }
    }

    fn merge(&mut self, other: Self) {
        match (self, other) {
            (scope, Self::All) => *scope = Self::All,
            (Self::Paths(paths), Self::Paths(incoming)) => {
                for path in incoming {
                    if !paths.contains(&path) {
                        paths.push(path);
                    }
                }
            }
            (Self::All, Self::Paths(_)) => {}
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RepoLoadsInFlight {
    in_flight: u32,
    pending: u32,
    contexts: FxHashMap<LoadKind, LoadContexts>,
    log: LogLoads,
    worktree_dirty: WorktreeDirtyLoads,
    line_stats: LineStatsLoads,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct LoadContexts {
    active: Option<ExecutionContext>,
    pending: Option<ExecutionContext>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct LogLoads {
    last_seq: LogLoadSeq,
    active: Option<ActiveLogLoad>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ActiveLogLoad {
    seq: LogLoadSeq,
    request: PendingLogLoad,
    pending: Option<PendingLogLoad>,
    context: Option<ExecutionContext>,
    pending_context: Option<ExecutionContext>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
enum WorktreeDirtyLoads {
    #[default]
    Idle,
    Running {
        pending: Option<WorktreeDirtyScope>,
        context: Option<ExecutionContext>,
        pending_context: Option<ExecutionContext>,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct LineStatsLoads {
    generation: LineStatsGeneration,
    active: Option<LineStatsGeneration>,
    requested: bool,
    active_context: Option<ExecutionContext>,
    requested_context: Option<ExecutionContext>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingLogLoad {
    pub scope: LogScope,
    pub author: Option<String>,
    pub limit: usize,
    pub cursor: Option<LogCursor>,
}

impl RepoLoadsInFlight {
    pub(crate) fn active_context(&self, kind: LoadKind) -> Option<ExecutionContext> {
        self.contexts
            .get(&kind)
            .and_then(|contexts| contexts.active.clone())
    }

    pub(crate) fn log_context(&self, seq: LogLoadSeq) -> Option<ExecutionContext> {
        self.log
            .active
            .as_ref()
            .filter(|active| active.seq == seq)
            .and_then(|active| active.context.clone())
    }

    pub(crate) fn worktree_context(&self) -> Option<ExecutionContext> {
        match &self.worktree_dirty {
            WorktreeDirtyLoads::Idle => None,
            WorktreeDirtyLoads::Running { context, .. } => context.clone(),
        }
    }

    pub(crate) fn line_stats_context(
        &self,
        generation: LineStatsGeneration,
    ) -> Option<ExecutionContext> {
        (self.line_stats.active == Some(generation))
            .then(|| self.line_stats.active_context.clone())
            .flatten()
    }

    const PRIMARY_REFRESH_FLAGS: u32 = LoadKind::HeadBranch.mask()
        | LoadKind::UpstreamDivergence.mask()
        | LoadKind::RebaseState.mask()
        | LoadKind::MergeCommitMessage.mask()
        | LoadKind::WorktreeStatus.mask()
        | LoadKind::StagedStatus.mask();

    pub fn is_in_flight(&self, kind: LoadKind) -> bool {
        (self.in_flight & kind.mask()) != 0
    }

    pub fn log_in_flight(&self) -> bool {
        self.log.active.is_some()
    }

    pub fn worktree_dirty_in_flight(&self) -> bool {
        matches!(self.worktree_dirty, WorktreeDirtyLoads::Running { .. })
    }

    pub fn line_stats_in_flight(&self) -> bool {
        self.line_stats.active.is_some()
    }

    /// Optional remote tag discovery and linked-checkout scans do not keep the
    /// active tab's spinner running after its local contents are available.
    pub fn any_foreground_in_flight(&self) -> bool {
        (self.in_flight & !LoadKind::RemoteTags.mask()) != 0
            || self.log_in_flight()
            || self.line_stats_in_flight()
    }

    pub fn any_in_flight(&self) -> bool {
        self.in_flight != 0
            || self.log_in_flight()
            || self.line_stats_in_flight()
            || self.worktree_dirty_in_flight()
    }

    pub fn clear(&mut self) {
        self.in_flight = 0;
        self.pending = 0;
        self.contexts.clear();
        self.log.active = None;
        self.worktree_dirty = WorktreeDirtyLoads::Idle;
        self.line_stats.invalidate();
        self.line_stats.active = None;
        self.line_stats.requested = false;
        self.line_stats.active_context = None;
        self.line_stats.requested_context = None;
    }

    pub(crate) fn invalidate_line_stats(&mut self) {
        self.line_stats.invalidate();
    }

    /// Retire an obsolete walk without requesting a replacement for an inactive tab.
    pub(crate) fn invalidate_log(&mut self) {
        self.log.active = None;
    }

    /// Counting reads both sides of each changed file, so it starts only after
    /// both status lanes and their replays settle on a fresh snapshot.
    pub(crate) fn start_line_stats(&mut self, status_ready: bool) -> Option<LineStatsGeneration> {
        if self.is_in_flight(LoadKind::WorktreeStatus) || self.is_in_flight(LoadKind::StagedStatus)
        {
            return None;
        }
        self.line_stats.start(status_ready)
    }

    /// Only the matching job may release the lane; invalidated results are discarded.
    pub(crate) fn finish_line_stats(&mut self, generation: LineStatsGeneration) -> bool {
        self.line_stats.finish(generation)
    }

    /// Starts the common primary-refresh batch only when all load protocols are idle.
    pub fn request_primary_refresh_batch(&mut self, log: PendingLogLoad) -> Option<LogLoadSeq> {
        if self.any_in_flight() || self.pending != 0 {
            return None;
        }
        self.in_flight |= Self::PRIMARY_REFRESH_FLAGS;
        for kind in [
            LoadKind::HeadBranch,
            LoadKind::UpstreamDivergence,
            LoadKind::RebaseState,
            LoadKind::MergeCommitMessage,
            LoadKind::WorktreeStatus,
            LoadKind::StagedStatus,
        ] {
            self.contexts.insert(
                kind,
                LoadContexts {
                    active: context::current(),
                    pending: None,
                },
            );
        }
        Some(self.log.start(log))
    }

    /// Starts immediately if idle, otherwise remembers one pending refresh.
    pub fn request(&mut self, kind: LoadKind) -> bool {
        if self.is_in_flight(kind) {
            self.pending |= kind.mask();
            self.contexts.entry(kind).or_default().pending = context::current();
            false
        } else {
            self.in_flight |= kind.mask();
            self.contexts.insert(
                kind,
                LoadContexts {
                    active: context::current(),
                    pending: None,
                },
            );
            true
        }
    }

    pub fn request_worktree_dirty(&mut self, scope: WorktreeDirtyScope) -> bool {
        match &mut self.worktree_dirty {
            WorktreeDirtyLoads::Idle => {
                self.worktree_dirty = WorktreeDirtyLoads::Running {
                    pending: None,
                    context: context::current(),
                    pending_context: None,
                };
                true
            }
            WorktreeDirtyLoads::Running {
                pending,
                pending_context,
                ..
            } => {
                *pending_context = context::current();
                match pending {
                    Some(pending) => pending.merge(scope),
                    None => *pending = Some(scope),
                }
                false
            }
        }
    }

    pub(crate) fn finish_worktree_dirty(&mut self) -> Option<WorktreeDirtyScope> {
        match &mut self.worktree_dirty {
            WorktreeDirtyLoads::Running {
                pending,
                context,
                pending_context,
            } if pending.is_some() => {
                *context = pending_context.take();
                pending.take()
            }
            WorktreeDirtyLoads::Idle | WorktreeDirtyLoads::Running { .. } => {
                self.worktree_dirty = WorktreeDirtyLoads::Idle;
                None
            }
        }
    }

    /// Finishes a load and indicates whether its pending refresh should start now.
    pub fn finish(&mut self, kind: LoadKind) -> bool {
        self.in_flight &= !kind.mask();
        if (self.pending & kind.mask()) != 0 {
            self.pending &= !kind.mask();
            self.in_flight |= kind.mask();
            if let Some(contexts) = self.contexts.get_mut(&kind) {
                contexts.active = contexts.pending.take();
            }
            true
        } else {
            self.contexts.remove(&kind);
            false
        }
    }

    /// A changed scope or author supersedes the current walk immediately.
    /// Otherwise one request is queued, with refresh taking priority over pagination.
    pub fn request_log(&mut self, next: PendingLogLoad) -> Option<LogLoadSeq> {
        self.log.request(next)
    }

    pub(crate) fn log_request_starts(&self, next: &PendingLogLoad) -> bool {
        self.log.request_starts(next)
    }

    pub fn is_active_log_reply(&self, seq: LogLoadSeq) -> bool {
        self.active_log_seq() == Some(seq)
    }

    pub fn active_log_seq(&self) -> Option<LogLoadSeq> {
        self.log.active.as_ref().map(|active| active.seq)
    }

    pub fn active_log_is_load_more(&self) -> bool {
        self.log
            .active
            .as_ref()
            .is_some_and(|active| active.request.cursor.is_some())
    }

    /// Only the matching walk may finish and promote a queued request.
    /// `prepare` runs before that request is recorded, keeping the effect and state equal.
    pub fn finish_log(
        &mut self,
        seq: LogLoadSeq,
        prepare: impl FnOnce(&mut PendingLogLoad),
    ) -> Option<(LogLoadSeq, PendingLogLoad)> {
        self.log.finish(seq, prepare)
    }
}

impl LogLoads {
    fn request_starts(&self, next: &PendingLogLoad) -> bool {
        self.active.as_ref().is_none_or(|active| {
            active.request.scope != next.scope || active.request.author != next.author
        })
    }

    fn start(&mut self, request: PendingLogLoad) -> LogLoadSeq {
        self.last_seq = LogLoadSeq(self.last_seq.0.wrapping_add(1));
        self.active = Some(ActiveLogLoad {
            seq: self.last_seq,
            request,
            pending: None,
            context: context::current(),
            pending_context: None,
        });
        self.last_seq
    }

    fn request(&mut self, next: PendingLogLoad) -> Option<LogLoadSeq> {
        if self.request_starts(&next) {
            return Some(self.start(next));
        }
        let active = self
            .active
            .as_mut()
            .expect("coalesced log request has an active walk");
        match &active.pending {
            Some(existing) if existing.cursor.is_none() && next.cursor.is_some() => {}
            _ => {
                active.pending = Some(next);
                active.pending_context = context::current();
            }
        }
        None
    }

    fn finish(
        &mut self,
        seq: LogLoadSeq,
        prepare: impl FnOnce(&mut PendingLogLoad),
    ) -> Option<(LogLoadSeq, PendingLogLoad)> {
        if self.active.as_ref().is_none_or(|active| active.seq != seq) {
            return None;
        }
        let active = self.active.take()?;
        let mut next = active.pending?;
        prepare(&mut next);
        let seq = self.start(next.clone());
        self.active
            .as_mut()
            .expect("promoted log request is active")
            .context = active.pending_context;
        Some((seq, next))
    }
}

impl LineStatsLoads {
    fn invalidate(&mut self) {
        self.generation = LineStatsGeneration(self.generation.0.wrapping_add(1));
        self.requested = true;
        self.requested_context = context::current();
    }

    fn start(&mut self, status_ready: bool) -> Option<LineStatsGeneration> {
        if self.active.is_some() || !self.requested {
            return None;
        }
        self.requested = false;
        if !status_ready {
            return None;
        }
        self.active = Some(self.generation);
        self.active_context = self.requested_context.take();
        Some(self.generation)
    }

    fn finish(&mut self, generation: LineStatsGeneration) -> bool {
        if self.active != Some(generation) {
            return false;
        }
        self.active = None;
        self.active_context = None;
        generation == self.generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalesced_loads_preserve_active_and_pending_action_contexts() {
        use gitcomet_core::git_execution_log::context::{ExecutionContext, UiSurface};
        let mut loads = RepoLoadsInFlight::default();
        let a = ExecutionContext::ui(UiSurface::Button, "a");
        let b = ExecutionContext::ui(UiSurface::CommandPalette, "b");
        let c = ExecutionContext::ui(UiSurface::Menu, "c");
        {
            let _scope = a.attach();
            assert!(loads.request(LoadKind::WorktreeStatus));
        }
        {
            let _scope = b.attach();
            assert!(!loads.request(LoadKind::WorktreeStatus));
        }
        assert_eq!(loads.active_context(LoadKind::WorktreeStatus), Some(a));
        assert!(loads.finish(LoadKind::WorktreeStatus));
        let scheduled = loads.active_context(LoadKind::WorktreeStatus);
        {
            let _scope = c.attach();
            assert!(!loads.request(LoadKind::WorktreeStatus));
        }
        assert_eq!(scheduled, Some(b.clone()));
        assert_eq!(loads.active_context(LoadKind::WorktreeStatus), Some(b));
        assert!(loads.finish(LoadKind::WorktreeStatus));
        assert_eq!(loads.active_context(LoadKind::WorktreeStatus), Some(c));
    }

    #[test]
    fn queued_history_refresh_keeps_its_context_when_pagination_is_rejected() {
        use gitcomet_core::git_execution_log::context::{ExecutionContext, UiSurface};
        let mut loads = RepoLoadsInFlight::default();
        let a = ExecutionContext::ui(UiSurface::Button, "a");
        let b = ExecutionContext::ui(UiSurface::Button, "b");
        let c = ExecutionContext::ui(UiSurface::Button, "c");
        let refresh = PendingLogLoad {
            scope: LogScope::FullReachable,
            author: None,
            limit: 20,
            cursor: None,
        };
        let first = {
            let _scope = a.attach();
            loads.request_log(refresh.clone()).unwrap()
        };
        {
            let _scope = b.attach();
            assert!(loads.request_log(refresh.clone()).is_none());
        }
        let page = PendingLogLoad {
            cursor: Some(LogCursor {
                last_seen: gitcomet_core::domain::CommitId("a".repeat(40).into()),
                resume_from: None,
                resume_token: None,
            }),
            ..refresh
        };
        {
            let _scope = c.attach();
            assert!(loads.request_log(page).is_none());
        }
        assert_eq!(loads.log_context(first), Some(a));
        let (next, _) = loads.finish_log(first, |_| {}).unwrap();
        assert_eq!(loads.log_context(next), Some(b));
        assert!(loads.log_context(first).is_none());
    }

    #[test]
    fn line_statistics_keep_the_requested_generations_context() {
        use gitcomet_core::git_execution_log::context::{ExecutionContext, UiSurface};
        let mut loads = RepoLoadsInFlight::default();
        let a = ExecutionContext::ui(UiSurface::Button, "a");
        let b = ExecutionContext::ui(UiSurface::Button, "b");
        {
            let _scope = a.attach();
            loads.invalidate_line_stats();
        }
        let first = loads.start_line_stats(true).unwrap();
        {
            let _scope = b.attach();
            loads.invalidate_line_stats();
        }
        assert_eq!(loads.line_stats_context(first), Some(a));
        assert!(!loads.finish_line_stats(first));
        let second = loads.start_line_stats(true).unwrap();
        assert_eq!(loads.line_stats_context(second), Some(b));
        assert!(loads.line_stats_context(first).is_none());
    }

    #[test]
    fn remote_tags_stay_tracked_without_delaying_local_readiness() {
        let mut loads = RepoLoadsInFlight::default();
        assert!(loads.request(LoadKind::RemoteTags));
        assert!(loads.any_in_flight());
        assert!(!loads.any_foreground_in_flight());

        assert!(loads.request(LoadKind::WorktreeStatus));
        assert!(loads.any_foreground_in_flight());
        assert!(!loads.finish(LoadKind::WorktreeStatus));
        assert!(!loads.any_foreground_in_flight());

        // Background classification must preserve coalescing and completion.
        assert!(!loads.request(LoadKind::RemoteTags));
        assert!(loads.finish(LoadKind::RemoteTags));
        assert!(loads.any_in_flight());
        assert!(!loads.any_foreground_in_flight());
        assert!(!loads.finish(LoadKind::RemoteTags));
        assert!(!loads.any_in_flight());
    }

    #[test]
    fn promoted_log_bookkeeping_matches_the_prepared_effect_request() {
        let mut loads = RepoLoadsInFlight::default();
        let request = PendingLogLoad {
            scope: LogScope::FullReachable,
            author: None,
            limit: 20,
            cursor: None,
        };
        let first = loads.request_log(request.clone()).unwrap();
        assert!(loads.request_log(request).is_none());
        let (seq, request) = loads.finish_log(first, |next| next.limit = 800).unwrap();
        assert_eq!(request.limit, 800);
        let active = loads.log.active.as_ref().unwrap();
        assert_eq!(active.seq, seq);
        assert_eq!(active.request, request);
    }
}
