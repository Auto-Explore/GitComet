//! Request-scoped progress for the commit-message editor.

use gitcomet_core::domain::CommitId;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RewordRequestId(u64);

impl RewordRequestId {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

impl Default for RewordRequestId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RewordStatus {
    Running,
    AwaitingAuthentication,
    Completed,
    Failed,
    Rejected,
    Cancelled,
}

impl RewordStatus {
    pub fn is_pending(self) -> bool {
        matches!(self, Self::Running | Self::AwaitingAuthentication)
    }
}

#[derive(Clone, Debug)]
pub struct RewordRequest {
    pub commit_id: CommitId,
    pub message: String,
    pub status: RewordStatus,
    /// The editor no longer needs a terminal acknowledgment. Running work
    /// and the identity of a paused rebase still outlive its editor.
    pub(crate) dismissed: bool,
}

impl RewordRequest {
    pub fn new(commit_id: CommitId, message: String, status: RewordStatus) -> Self {
        Self {
            commit_id,
            message,
            status,
            dismissed: false,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct RewordState {
    pub requests: Arc<BTreeMap<RewordRequestId, RewordRequest>>,
    pub revision: u64,
    /// Retained even after an authentication prompt is dismissed, so manual
    /// Continue still completes the original edit.
    pub(crate) continuation: Option<RewordRequestId>,
}

impl RewordState {
    pub fn can_resume(&self, id: RewordRequestId) -> bool {
        self.continuation == Some(id)
    }

    /// A partially completed rewrite can remove its original target from
    /// history while its editor still owns the selection.
    pub(crate) fn retains_selected_commit(&self, commit_id: &CommitId) -> bool {
        self.requests.iter().any(|(id, request)| {
            &request.commit_id == commit_id
                && (request.status.is_pending() || self.continuation == Some(*id))
        })
    }

    pub(crate) fn insert(&mut self, id: RewordRequestId, request: RewordRequest) {
        Arc::make_mut(&mut self.requests).insert(id, request);
        self.revision = self.revision.wrapping_add(1);
    }

    pub(crate) fn set_status(&mut self, id: RewordRequestId, status: RewordStatus) {
        let Some(request) = Arc::make_mut(&mut self.requests).get_mut(&id) else {
            return;
        };
        request.status = status;
        self.revision = self.revision.wrapping_add(1);
        self.prune_dismissed(id);
    }

    pub(crate) fn dismiss(&mut self, id: RewordRequestId) {
        if let Some(request) = Arc::make_mut(&mut self.requests).get_mut(&id) {
            request.dismissed = true;
            self.revision = self.revision.wrapping_add(1);
            self.prune_dismissed(id);
        }
    }

    pub(crate) fn clear_continuation(&mut self) {
        if let Some(id) = self.continuation.take() {
            self.set_status(id, RewordStatus::Cancelled);
        }
    }

    fn prune_dismissed(&mut self, id: RewordRequestId) {
        if self.continuation != Some(id)
            && self
                .requests
                .get(&id)
                .is_some_and(|request| request.dismissed && !request.status.is_pending())
        {
            Arc::make_mut(&mut self.requests).remove(&id);
        }
    }
}
