use crate::domain::{CommitId, Worktree, WorktreeDirtySummary};
use crate::error::Error;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorktreeCleanliness {
    Clean,
    Dirty,
    Unknown(Arc<str>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorktreeObservation {
    Clean(Worktree),
    Dirty(WorktreeDirtySummary),
    Unknown { worktree: Worktree, error: Arc<str> },
}

impl WorktreeObservation {
    pub fn path(&self) -> &Path {
        match self {
            Self::Clean(worktree) | Self::Unknown { worktree, .. } => &worktree.path,
            Self::Dirty(summary) => &summary.path,
        }
    }

    pub fn head(&self) -> Option<&CommitId> {
        match self {
            Self::Clean(worktree) | Self::Unknown { worktree, .. } => worktree.head.as_ref(),
            Self::Dirty(summary) => summary.head.as_ref(),
        }
    }

    pub fn cleanliness(&self) -> WorktreeCleanliness {
        match self {
            Self::Clean(_) => WorktreeCleanliness::Clean,
            Self::Dirty(_) => WorktreeCleanliness::Dirty,
            Self::Unknown { error, .. } => WorktreeCleanliness::Unknown(error.clone()),
        }
    }
}

#[derive(Debug)]
pub struct WorktreeScan {
    pub id: u64,
    pub outcome: Result<Vec<WorktreeObservation>, Error>,
}

impl WorktreeScan {
    pub fn new(outcome: Result<Vec<WorktreeObservation>, Error>) -> Self {
        static NEXT_SCAN: AtomicU64 = AtomicU64::new(1);
        Self {
            id: NEXT_SCAN.fetch_add(1, Ordering::Relaxed),
            outcome,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn from_dirty_for_test(result: Result<Vec<WorktreeDirtySummary>, Error>) -> Self {
        Self::new(result.map(|summaries| {
            summaries
                .into_iter()
                .map(WorktreeObservation::Dirty)
                .collect()
        }))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservedWorktreeStatus {
    pub scan_id: u64,
    pub head: Option<CommitId>,
    pub cleanliness: WorktreeCleanliness,
}
