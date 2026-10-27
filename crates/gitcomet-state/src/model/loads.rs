//! Per-repository load coalescing and log-walk sequencing.

use gitcomet_core::domain::LogCursor;
use std::path::{Path, PathBuf};

/// Loads that coalesce repeated requests into one pending refresh.
/// Log walks, linked-checkout scans, and line statistics have separate protocols.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
    log: LogLoads,
    worktree_dirty: WorktreeDirtyLoads,
    line_stats: LineStatsLoads,
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
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
enum WorktreeDirtyLoads {
    #[default]
    Idle,
    Running {
        pending: Option<WorktreeDirtyScope>,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct LineStatsLoads {
    generation: LineStatsGeneration,
    active: Option<LineStatsGeneration>,
    requested: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingLogLoad {
    pub walk: gitcomet_core::services::HistoryWalk,
    pub author: Option<String>,
    /// The refs the walk is focused on, empty for the normal seed.
    pub focus: gitcomet_core::domain::HistoryFocusSet,
    pub limit: usize,
    pub cursor: Option<LogCursor>,
}

impl RepoLoadsInFlight {
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
        self.log.active = None;
        self.worktree_dirty = WorktreeDirtyLoads::Idle;
        self.line_stats.invalidate();
        self.line_stats.active = None;
        self.line_stats.requested = false;
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
        Some(self.log.start(log))
    }

    /// Starts immediately if idle, otherwise remembers one pending refresh.
    pub fn request(&mut self, kind: LoadKind) -> bool {
        if self.is_in_flight(kind) {
            self.pending |= kind.mask();
            false
        } else {
            self.in_flight |= kind.mask();
            true
        }
    }

    pub fn request_worktree_dirty(&mut self, scope: WorktreeDirtyScope) -> bool {
        match &mut self.worktree_dirty {
            WorktreeDirtyLoads::Idle => {
                self.worktree_dirty = WorktreeDirtyLoads::Running { pending: None };
                true
            }
            WorktreeDirtyLoads::Running { pending } => {
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
            WorktreeDirtyLoads::Running { pending } if pending.is_some() => pending.take(),
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
            true
        } else {
            false
        }
    }

    /// A changed walk, author or focus supersedes the current walk immediately.
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
            active.request.walk != next.walk
                || active.request.author != next.author
                || active.request.focus != next.focus
        })
    }

    fn start(&mut self, request: PendingLogLoad) -> LogLoadSeq {
        self.last_seq = LogLoadSeq(self.last_seq.0.wrapping_add(1));
        self.active = Some(ActiveLogLoad {
            seq: self.last_seq,
            request,
            pending: None,
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
            _ => active.pending = Some(next),
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
        let mut next = self.active.take()?.pending?;
        prepare(&mut next);
        let seq = self.start(next.clone());
        Some((seq, next))
    }
}

impl LineStatsLoads {
    fn invalidate(&mut self) {
        self.generation = LineStatsGeneration(self.generation.0.wrapping_add(1));
        self.requested = true;
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
        Some(self.generation)
    }

    fn finish(&mut self, generation: LineStatsGeneration) -> bool {
        if self.active != Some(generation) {
            return false;
        }
        self.active = None;
        generation == self.generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::LogScope;

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
            focus: Default::default(),
            walk: LogScope::FullReachable.into(),
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
