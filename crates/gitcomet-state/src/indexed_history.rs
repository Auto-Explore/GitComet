//! Independent generations and a bounded cache for indexed history requests.
use crate::model::RepoId;
use gitcomet_core::domain::HistoryMode;
use gitcomet_core::history_index::{HistoryIndexHandle, HistoryIndexProgress, HistoryRange};
use gitcomet_core::services::{CancellationToken, HistorySnapshot, Result};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub struct IndexedHistoryState {
    pub index: Option<HistoryIndexHandle>,
    /// The presentation currently published by the history view. Range requests
    /// may already belong to a replacement while these rows remain interactive.
    pub displayed_index: Option<HistoryIndexHandle>,
    pub range_index: Option<HistoryIndexHandle>,
    pub requested: Option<HistorySnapshot>,
    pub loading: bool,
    pub progress: Option<HistoryIndexProgress>,
    pub error: Option<String>,
    pub ranges: BTreeMap<usize, Arc<HistoryRange>>,
    pub range_errors: BTreeMap<usize, String>,
    pub rev: u64,
    pub epoch: u64,
    pub(crate) seq: u64,
    pub(crate) cancellation: CancellationToken,
    pub(crate) desired: Vec<usize>,
    pub(crate) pending: BTreeMap<usize, (u64, CancellationToken)>,
    pub(crate) range_seq: u64,
    pub(crate) lru: VecDeque<usize>,
}

impl IndexedHistoryState {
    pub fn commit(
        &self,
        snapshot: &HistorySnapshot,
        row: usize,
    ) -> Option<&gitcomet_core::domain::Commit> {
        if self
            .range_index
            .as_ref()
            .is_none_or(|index| &index.snapshot != snapshot)
        {
            return None;
        }
        let block = row / gitcomet_core::history_index::HISTORY_BLOCK_SIZE
            * gitcomet_core::history_index::HISTORY_BLOCK_SIZE;
        self.ranges.get(&block)?.commits.get(row - block)
    }
    pub fn reset_query(&mut self) {
        self.cancel();
        let seq = self.seq.wrapping_add(1);
        let range_seq = self.range_seq.wrapping_add(1);
        let rev = self.rev.wrapping_add(1);
        *self = Self {
            seq,
            range_seq,
            rev,
            ..Default::default()
        };
    }
    pub fn cancel(&mut self) {
        self.cancellation.cancel();
        for (_, token) in self.pending.values() {
            token.cancel();
        }
        self.pending.clear();
    }
}

#[derive(Debug)]
pub enum IndexedHistoryMsg {
    Retry {
        repo_id: RepoId,
    },
    Ensure {
        repo_id: RepoId,
    },
    Publish {
        repo_id: RepoId,
        index: HistoryIndexHandle,
    },
    Select {
        repo_id: RepoId,
        commit_id: gitcomet_core::domain::CommitId,
        mode: crate::msg::CommitSelectMode,
        projection: gitcomet_core::history_index::HistoryProjection,
    },
    RequestRanges {
        repo_id: RepoId,
        snapshot: HistorySnapshot,
        blocks: Vec<usize>,
        retry: bool,
    },
    Progress {
        repo_id: RepoId,
        seq: u64,
        progress: HistoryIndexProgress,
    },
    Built {
        repo_id: RepoId,
        seq: u64,
        result: Result<Option<HistoryIndexHandle>>,
    },
    RangeLoaded {
        repo_id: RepoId,
        snapshot: HistorySnapshot,
        seq: u64,
        start: usize,
        result: Result<HistoryRange>,
    },
}

#[derive(Clone, Debug)]
pub enum IndexedHistoryEffect {
    Build {
        repo_id: RepoId,
        seq: u64,
        mode: HistoryMode,
        author: Option<String>,
        cancellation: CancellationToken,
    },
    Range {
        repo_id: RepoId,
        seq: u64,
        index: HistoryIndexHandle,
        start: usize,
        cancellation: CancellationToken,
    },
}

impl IndexedHistoryEffect {
    pub fn repo_id(&self) -> RepoId {
        match self {
            Self::Build { repo_id, .. } | Self::Range { repo_id, .. } => *repo_id,
        }
    }
    pub fn failed(self, error: gitcomet_core::error::Error) -> IndexedHistoryMsg {
        match self {
            Self::Build { repo_id, seq, .. } => IndexedHistoryMsg::Built {
                repo_id,
                seq,
                result: Err(error),
            },
            Self::Range {
                repo_id,
                seq,
                index,
                start,
                ..
            } => IndexedHistoryMsg::RangeLoaded {
                repo_id,
                seq,
                snapshot: index.snapshot.clone(),
                start,
                result: Err(error),
            },
        }
    }
}
