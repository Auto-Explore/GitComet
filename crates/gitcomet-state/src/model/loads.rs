//! Per-repository load coalescing and log-walk sequencing.

use gitcomet_core::domain::{LogCursor, LogScope};
use std::path::{Path, PathBuf};

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
    pending_log: Option<PendingLogLoad>,
    pending_worktree_dirty: Option<WorktreeDirtyScope>,
    /// The log walk that is actually running, so replies from one a newer
    /// request superseded can be told apart from the current one.
    pub(super) active_log: Option<(LogLoadSeq, PendingLogLoad)>,
    last_log_seq: LogLoadSeq,
    line_stats_generation: LineStatsGeneration,
    active_line_stats: Option<LineStatsGeneration>,
    line_stats_requested: bool,
}

/// Identifies one dispatched log walk. Handed out by
/// [`RepoLoadsInFlight::request_log`] and carried by the effect and its replies,
/// so a reply is matched to the request that started it and nothing else.
///
/// A walk cannot be identified by what it asks for: switching the filter away
/// and back leaves the second request looking exactly like the first, and the
/// first walk's reply would then be taken for the second's — clearing the
/// bookkeeping while the walk it belongs to is still running.
pub type LogLoadSeq = u64;

/// Advances on invalidation, even when the set of changed paths is unchanged.
pub type LineStatsGeneration = u64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingLogLoad {
    pub scope: LogScope,
    pub author: Option<String>,
    pub limit: usize,
    pub cursor: Option<LogCursor>,
}

impl RepoLoadsInFlight {
    pub const HEAD_BRANCH: u32 = 1 << 0;
    pub const UPSTREAM_DIVERGENCE: u32 = 1 << 1;
    pub const BRANCHES: u32 = 1 << 2;
    pub const TAGS: u32 = 1 << 3;
    pub const REMOTES: u32 = 1 << 4;
    pub const REMOTE_BRANCHES: u32 = 1 << 5;
    pub const WORKTREE_STATUS: u32 = 1 << 6;
    pub const STAGED_STATUS: u32 = 1 << 7;
    pub const STASHES: u32 = 1 << 8;
    pub const REFLOG: u32 = 1 << 9;
    pub const REBASE_STATE: u32 = 1 << 10;
    pub const LOG: u32 = 1 << 11;
    pub const MERGE_COMMIT_MESSAGE: u32 = 1 << 12;
    pub const REMOTE_TAGS: u32 = 1 << 13;
    pub const WORKTREES: u32 = 1 << 14;
    pub const SUBMODULES: u32 = 1 << 15;
    pub const REF_METADATA: u32 = 1 << 16;
    pub const WORKTREE_DIRTY: u32 = 1 << 17;
    /// Deliberately outside `PRIMARY_REFRESH_FLAGS`: the live listing is a
    /// worktree walk, far costlier than the other loads.
    pub const FILE_BROWSER: u32 = 1 << 18;
    /// Also outside `PRIMARY_REFRESH_FLAGS`: counting reads both sides of every
    /// changed file, which a stat-only status walk avoids. Kept separate so status
    /// latency is unchanged and the numbers arrive after the list.
    /// Managed by `invalidate_line_stats`/`start_line_stats`/`finish_line_stats`,
    /// not generic `request`/`finish`: replays need a fresh status snapshot.
    pub const UNCOMMITTED_LINE_STATS: u32 = 1 << 19;
    /// Git LFS / git-annex repository facts. Outside the primary refresh:
    /// config and attributes change far less often than status.
    pub const LARGE_FILE_SUPPORT: u32 = 1 << 20;
    /// Git LFS lock list; a server round trip, loaded only on demand.
    pub const LFS_LOCKS: u32 = 1 << 21;
    /// `git annex unused`: one scan at a time, as every run rewrites the
    /// numbering that `dropunused` reads.
    pub const ANNEX_UNUSED: u32 = 1 << 22;
    const PRIMARY_REFRESH_FLAGS: u32 = Self::HEAD_BRANCH
        | Self::UPSTREAM_DIVERGENCE
        | Self::REBASE_STATE
        | Self::MERGE_COMMIT_MESSAGE
        | Self::WORKTREE_STATUS
        | Self::STAGED_STATUS
        | Self::LOG;

    pub fn is_in_flight(&self, flag: u32) -> bool {
        (self.in_flight & flag) != 0
    }

    pub fn any_in_flight(&self) -> bool {
        self.in_flight != 0
    }

    pub fn clear(&mut self) {
        self.in_flight = 0;
        self.pending = 0;
        self.pending_log = None;
        self.pending_worktree_dirty = None;
        self.active_log = None;
        self.line_stats_generation = self.line_stats_generation.wrapping_add(1);
        self.active_line_stats = None;
        self.line_stats_requested = false;
    }

    pub(crate) fn invalidate_line_stats(&mut self) {
        self.line_stats_generation = self.line_stats_generation.wrapping_add(1);
        self.line_stats_requested = true;
    }

    /// Retire an obsolete walk without requesting a replacement for an
    /// inactive tab. Its late replies must not refill the old history page.
    pub(crate) fn invalidate_log(&mut self) {
        self.in_flight &= !Self::LOG;
        self.pending &= !Self::LOG;
        self.pending_log = None;
        self.active_log = None;
    }

    /// Called only after both status lanes, including their replays, settle.
    pub(crate) fn start_line_stats(&mut self, status_ready: bool) -> Option<LineStatsGeneration> {
        if self.is_in_flight(Self::WORKTREE_STATUS | Self::STAGED_STATUS)
            || self.active_line_stats.is_some()
            || !self.line_stats_requested
        {
            return None;
        }
        self.line_stats_requested = false;
        if !status_ready {
            return None;
        }
        self.in_flight |= Self::UNCOMMITTED_LINE_STATS;
        self.active_line_stats = Some(self.line_stats_generation);
        Some(self.line_stats_generation)
    }

    /// Only the matching job may release the lane; invalidated results are discarded.
    pub(crate) fn finish_line_stats(&mut self, generation: LineStatsGeneration) -> bool {
        if self.active_line_stats != Some(generation) {
            return false;
        }
        self.active_line_stats = None;
        self.in_flight &= !Self::UNCOMMITTED_LINE_STATS;
        generation == self.line_stats_generation
    }

    /// Starts the common primary-refresh batch immediately when no work is already queued or
    /// running. Callers fall back to per-load request coalescing when this returns `None`.
    ///
    /// The batch includes a log load, so it takes that request and returns its
    /// sequence number: replies are matched against it by
    /// [`Self::is_active_log_reply`], and a batch that failed to declare one
    /// would have its log page silently discarded.
    pub fn request_primary_refresh_batch(&mut self, log: PendingLogLoad) -> Option<LogLoadSeq> {
        if self.in_flight == 0 && self.pending == 0 && self.pending_log.is_none() {
            self.in_flight |= Self::PRIMARY_REFRESH_FLAGS;
            Some(self.start_log(log))
        } else {
            None
        }
    }

    /// Marks `load` as the walk now in flight and hands out its sequence number.
    fn start_log(&mut self, load: PendingLogLoad) -> LogLoadSeq {
        self.last_log_seq = self.last_log_seq.wrapping_add(1);
        self.active_log = Some((self.last_log_seq, load));
        self.last_log_seq
    }

    /// For non-log loads: starts immediately if not in flight, otherwise coalesces by remembering
    /// one pending refresh for the same kind.
    pub fn request(&mut self, flag: u32) -> bool {
        if self.is_in_flight(flag) {
            self.pending |= flag;
            false
        } else {
            self.in_flight |= flag;
            true
        }
    }

    pub(crate) fn request_worktree_dirty(&mut self, scope: WorktreeDirtyScope) -> bool {
        if self.request(Self::WORKTREE_DIRTY) {
            true
        } else {
            match &mut self.pending_worktree_dirty {
                Some(pending) => pending.merge(scope),
                None => self.pending_worktree_dirty = Some(scope),
            }
            false
        }
    }

    pub(crate) fn finish_worktree_dirty(&mut self) -> Option<WorktreeDirtyScope> {
        if self.finish(Self::WORKTREE_DIRTY) {
            Some(self.pending_worktree_dirty.take().unwrap_or_default())
        } else {
            self.pending_worktree_dirty = None;
            None
        }
    }

    /// For non-log loads: finishes and indicates whether a pending request should be scheduled now.
    pub fn finish(&mut self, flag: u32) -> bool {
        self.in_flight &= !flag;
        if (self.pending & flag) != 0 {
            self.pending &= !flag;
            self.in_flight |= flag;
            true
        } else {
            false
        }
    }

    /// For log loads: coalesce by keeping only the latest requested
    /// `(scope, author, cursor)` while a log load is already in flight. Returns
    /// the new walk's sequence number when it starts now, `None` when it was
    /// queued behind the walk in flight.
    ///
    /// A request that changes the scope or the author filter is dispatched
    /// straight away instead of being queued: on a large repository a walk runs
    /// for tens of seconds, and the repo-load pool has one or two threads, so
    /// waiting the old one out would stall the new filter for that whole time.
    /// The effects layer cancels the superseded walk, and its reply is dropped
    /// by [`Self::is_active_log_reply`].
    pub fn request_log(&mut self, next: PendingLogLoad) -> Option<LogLoadSeq> {
        if !self.is_in_flight(Self::LOG) {
            self.in_flight |= Self::LOG;
            return Some(self.start_log(next));
        }

        let supersedes_active = self
            .active_log
            .as_ref()
            .is_none_or(|(_, active)| active.scope != next.scope || active.author != next.author);
        if supersedes_active {
            self.pending_log = None;
            return Some(self.start_log(next));
        }
        match &self.pending_log {
            // Scope or author changes invalidate older pending requests
            // (including pagination).
            Some(existing) if existing.scope != next.scope || existing.author != next.author => {
                self.pending_log = Some(next);
            }
            // A fresh walk invalidates pagination cursors. Never let a later
            // pagination request replace a pending refresh.
            Some(existing) if existing.cursor.is_none() && next.cursor.is_some() => {}
            _ => {
                self.pending_log = Some(next);
            }
        }
        None
    }

    /// Whether a log reply belongs to the walk that is currently in flight,
    /// rather than one that a newer request superseded (and that the effects
    /// layer cancelled). Superseded replies must be dropped without touching
    /// the in-flight bookkeeping — the walk that replaced them is still going.
    pub fn is_active_log_reply(&self, seq: LogLoadSeq) -> bool {
        self.active_log
            .as_ref()
            .is_some_and(|(active, _)| *active == seq)
    }

    /// The sequence number of the walk in flight, if any. Tests that answer a
    /// dispatched load by hand need it to send a reply the reducer will accept.
    pub fn active_log_seq(&self) -> Option<LogLoadSeq> {
        self.active_log.as_ref().map(|(seq, _)| *seq)
    }

    /// Whether the walk in flight is paginating rather than rebuilding the page.
    pub fn active_log_is_load_more(&self) -> bool {
        self.active_log
            .as_ref()
            .is_some_and(|(_, active)| active.cursor.is_some())
    }

    /// Finishes the walk in flight and starts whichever request queued behind
    /// it, returning that request and its sequence number. `prepare` adjusts
    /// the request before it is recorded, so the effect and bookkeeping agree.
    pub fn finish_log(
        &mut self,
        prepare: impl FnOnce(&mut PendingLogLoad),
    ) -> Option<(LogLoadSeq, PendingLogLoad)> {
        self.in_flight &= !Self::LOG;
        self.active_log = None;
        let mut next = self.pending_log.take()?;
        prepare(&mut next);
        self.in_flight |= Self::LOG;
        let seq = self.start_log(next.clone());
        Some((seq, next))
    }
}
