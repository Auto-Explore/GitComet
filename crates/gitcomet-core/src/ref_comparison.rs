use crate::domain::CommitId;
use std::time::Duration;

pub const MAX_COMPARISON_SUBJECTS: usize = 128;
pub const COMPARISON_ALGORITHM_VERSION: u32 = 1;
pub const COMPARISON_REPOSITORY_BYTES: usize = 32 * 1024 * 1024;
pub const COMPARISON_GLOBAL_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RefComparisonTarget {
    pub reference: String,
    pub oid: CommitId,
    pub context: u64,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RefComparisonDepth {
    Counts,
    Evidence,
    Deep,
}

#[derive(Clone, Copy, Debug)]
pub struct RefComparisonBudget {
    pub elapsed: Duration,
    pub memory_bytes: usize,
    pub history_commits: usize,
}

impl RefComparisonBudget {
    pub fn automatic() -> Self {
        Self {
            elapsed: Duration::from_secs(5),
            memory_bytes: COMPARISON_REPOSITORY_BYTES / 2,
            history_commits: 1_000,
        }
    }

    pub fn deep() -> Self {
        Self {
            elapsed: Duration::from_secs(30),
            history_commits: 10_000,
            ..Self::automatic()
        }
    }
}

#[derive(Clone, Debug)]
pub struct RefComparisonRequest {
    pub target: RefComparisonTarget,
    pub subjects: Vec<CommitId>,
    pub depth: RefComparisonDepth,
    pub budget: RefComparisonBudget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefComparisonReason {
    Budget,
    Deadline,
    IncompleteHistory,
    Unsupported,
    MultipleMergeBases,
    NoMergeBase,
    UniqueMerge,
    NoMatch,
}

impl RefComparisonReason {
    pub fn description(self) -> &'static str {
        match self {
            Self::Budget => {
                "The analysis reached its resource or history limit. Run a deeper check to search further."
            }
            Self::Deadline => {
                "The analysis reached its time limit. Run a deeper check to search further."
            }
            Self::IncompleteHistory => {
                "The locally available history is incomplete or changed during analysis."
            }
            Self::Unsupported => {
                "This backend or graph context does not support comparison analysis."
            }
            Self::MultipleMergeBases => "Multiple merge bases prevent a strict content comparison.",
            Self::NoMergeBase => "The histories have no common ancestor.",
            Self::UniqueMerge => "Unique merge commits prevent individual patch confirmation.",
            Self::NoMatch => "The available checks did not establish incorporation.",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefComparisonCounts {
    Pending,
    Exact { ahead: u64, behind: u64 },
    Unavailable(RefComparisonReason),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntegrationEvidence {
    UpToDate,
    Merged,
    ChangesMatch,
    SquashPatch,
    IndividualCommits,
    NoNetChanges,
}

impl IntegrationEvidence {
    pub fn label(self) -> &'static str {
        match self {
            Self::UpToDate => "Up to date",
            Self::Merged => "Merged",
            Self::ChangesMatch => "Changes match",
            Self::SquashPatch | Self::IndividualCommits => "Equivalent history",
            Self::NoNetChanges => "No net changes",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::UpToDate => "The subject and target identify the same commit.",
            Self::Merged => "The subject's committed history is reachable from the target.",
            Self::ChangesMatch => {
                "Every path changed since the merge base has the same final Git object and mode, or deletion, in the target."
            }
            Self::SquashPatch => {
                "An ordinary target-history commit matches the aggregate branch patch (squash-patch evidence). This is historical evidence; it does not assert present-day content or identical history."
            }
            Self::IndividualCommits => {
                "Every unique ordinary branch commit has a distinct historical patch match (individual-commit evidence). This does not assert present-day content or identical history."
            }
            Self::NoNetChanges => {
                "The branch has no final changes relative to its merge base, despite possibly having unique commits."
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefIntegration {
    Pending,
    Confirmed(IntegrationEvidence),
    NotConfirmed(RefComparisonReason),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RefComparison {
    pub subject: CommitId,
    pub counts: RefComparisonCounts,
    pub integration: RefIntegration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RefComparisonBatch {
    pub target: RefComparisonTarget,
    pub results: Vec<RefComparison>,
    pub graph_reads: u64,
    pub cache_hits: usize,
}
