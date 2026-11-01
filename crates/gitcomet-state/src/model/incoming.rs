use gitcomet_core::domain::HeadTracking;
use std::time::SystemTime;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepoIncomingSnapshot {
    pub tracking: HeadTracking,
    pub checked_at: SystemTime,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum RepoIncomingState {
    #[default]
    Unknown,
    Current(RepoIncomingSnapshot),
    Stale(RepoIncomingSnapshot),
}

impl RepoIncomingState {
    pub(crate) fn mark_stale(&mut self) {
        if let Self::Current(snapshot) = self {
            *self = Self::Stale(snapshot.clone());
        }
    }
}
