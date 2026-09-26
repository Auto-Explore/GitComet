//! Find-in-history over every row of an indexed history, not only the rows
//! whose text the viewport has loaded.
use crate::model::RepoId;
use gitcomet_core::history_find::HistoryFindQuery;
use gitcomet_core::history_index::HistoryIndexHandle;
use gitcomet_core::services::{CancellationToken, Result};
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub struct HistoryFindState {
    pub query: Option<HistoryFindQuery>,
    /// The index the matches belong to. Results for any other index are stale.
    pub index: Option<HistoryIndexHandle>,
    /// Matching raw index rows in ascending (display) order.
    pub matches: Arc<Vec<usize>>,
    pub done: bool,
    pub error: Option<String>,
    pub rev: u64,
    pub(crate) seq: u64,
    pub(crate) cancellation: CancellationToken,
}

impl HistoryFindState {
    /// Repository loads were cancelled (a tab switch, a finished action, a
    /// reload). The scan's reply is dropped with them, so an unfinished search
    /// would otherwise wait forever; clearing it lets the view ask again.
    /// Finished results stay, since they still describe their index.
    pub(crate) fn interrupt(&mut self) {
        self.cancellation.cancel();
        if self.query.is_some() && !self.done && self.error.is_none() {
            *self = Self {
                seq: self.seq.wrapping_add(1),
                rev: self.rev.wrapping_add(1),
                ..Self::default()
            };
        }
    }

    /// Whether these results answer `query` over `index`.
    pub fn is_for(&self, query: &HistoryFindQuery, index: &HistoryIndexHandle) -> bool {
        self.query.as_ref() == Some(query)
            && self
                .index
                .as_ref()
                .is_some_and(|known| Arc::ptr_eq(known, index))
    }
}

#[derive(Clone, Debug)]
pub struct HistoryFindChunk {
    /// Newly found raw rows, all greater than any row reported before.
    pub matches: Vec<usize>,
    pub done: bool,
}

#[derive(Debug)]
pub enum HistoryFindMsg {
    /// Start a search, or with `query: None` stop searching and clear results.
    Find {
        repo_id: RepoId,
        query: Option<HistoryFindQuery>,
        index: Option<HistoryIndexHandle>,
    },
    Found {
        repo_id: RepoId,
        seq: u64,
        result: Result<HistoryFindChunk>,
    },
}

#[derive(Clone, Debug)]
pub struct HistoryFindEffect {
    pub repo_id: RepoId,
    pub seq: u64,
    pub index: HistoryIndexHandle,
    pub query: HistoryFindQuery,
    pub cancellation: CancellationToken,
}

impl HistoryFindEffect {
    pub fn failed(self, error: gitcomet_core::error::Error) -> HistoryFindMsg {
        HistoryFindMsg::Found {
            repo_id: self.repo_id,
            seq: self.seq,
            result: Err(error),
        }
    }
}
