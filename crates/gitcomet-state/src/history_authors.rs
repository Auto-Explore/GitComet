//! On-demand author discovery, independent of history paging and author filters.
use crate::model::{Loadable, RepoId, RepoState};
use gitcomet_core::domain::HistoryMode;
use gitcomet_core::services::{CancellationToken, HistorySnapshot, Result};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct HistoryAuthorsState {
    pub names: Loadable<Arc<[Arc<str>]>>,
    pub rev: u64,
    pub(crate) source: Option<(HistoryMode, Option<HistorySnapshot>, u64)>,
    pub(crate) seq: u64,
    pub(crate) cancellation: CancellationToken,
}

impl Default for HistoryAuthorsState {
    fn default() -> Self {
        Self {
            names: Loadable::NotLoaded,
            rev: 0,
            source: None,
            seq: 0,
            cancellation: CancellationToken::new(),
        }
    }
}

impl HistoryAuthorsState {
    #[cfg(feature = "test-support")]
    pub fn loaded_for_test(repo: &RepoState, names: Arc<[Arc<str>]>) -> Self {
        Self {
            names: Loadable::Ready(names),
            rev: 1,
            source: Some(Self::source_for(repo)),
            ..Self::default()
        }
    }

    pub fn needs_load(&self, repo: &RepoState) -> bool {
        self.source.as_ref() != Some(&Self::source_for(repo)) || self.cancellation.is_cancelled()
    }

    pub fn matches_scope(&self, scope: HistoryMode) -> bool {
        self.source
            .as_ref()
            .is_some_and(|(mode, ..)| *mode == scope)
    }

    pub(crate) fn source_for(repo: &RepoState) -> (HistoryMode, Option<HistorySnapshot>, u64) {
        (
            repo.history_state.history_scope,
            repo.history_state.log_snapshot.clone(),
            repo.load_epoch,
        )
    }
}

#[derive(Debug)]
pub enum HistoryAuthorsMsg {
    Ensure {
        repo_id: RepoId,
        retry: bool,
    },
    Loaded {
        repo_id: RepoId,
        seq: u64,
        result: Result<Arc<[Arc<str>]>>,
    },
}

#[derive(Clone, Debug)]
pub struct HistoryAuthorsEffect {
    pub repo_id: RepoId,
    pub seq: u64,
    pub mode: HistoryMode,
    pub cancellation: CancellationToken,
}

impl HistoryAuthorsEffect {
    pub fn failed(self, error: gitcomet_core::error::Error) -> HistoryAuthorsMsg {
        HistoryAuthorsMsg::Loaded {
            repo_id: self.repo_id,
            seq: self.seq,
            result: Err(error),
        }
    }
}
