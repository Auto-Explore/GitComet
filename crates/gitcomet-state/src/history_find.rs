//! Find-in-history over every row of an indexed history, not only the rows
//! whose text the viewport has loaded.
use crate::model::RepoId;
use gitcomet_core::history_find::HistoryFindQuery;
use gitcomet_core::history_index::{HistoryIndex, HistoryIndexHandle};
use gitcomet_core::services::{CancellationToken, Result};
use std::sync::{Arc, Mutex, Weak};

mod cache;
use cache::HistoryFindCache;

/// Thread-name prefix of the scan worker, so its reads are identifiable.
pub const HISTORY_FIND_THREAD: &str = "gitcomet-history-find";

#[derive(Clone, Debug, Default)]
pub struct HistoryFindState {
    pub query: Option<HistoryFindQuery>,
    pub stashes_rev: u64,
    /// Identity only: finished results must not keep obsolete indexes alive.
    pub index: Option<Weak<HistoryIndex>>,
    /// Matching raw index rows in ascending (display) order, in the chunks
    /// the scan reported them. A search only ever appends chunks, so a reader
    /// can resume after the ones it has already seen, and publishing a new
    /// chunk copies the chunk list rather than every match found so far.
    pub matches: Arc<Vec<Arc<[u32]>>>,
    pub done: bool,
    pub error: Option<String>,
    pub rev: u64,
    pub(crate) seq: u64,
    pub(crate) cancellation: CancellationToken,
    pub(crate) cache: Arc<Mutex<HistoryFindCache>>,
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
                cache: Arc::clone(&self.cache),
                ..Self::default()
            };
        }
    }

    /// The find bar closed: stop searching and free the commit text.
    pub(crate) fn close(&mut self) {
        self.cancellation.cancel();
        *self = Self {
            seq: self.seq.wrapping_add(1),
            rev: self.rev.wrapping_add(1),
            ..Self::default()
        };
    }

    /// Identifies one search; its `matches` only grow while it lasts.
    pub fn generation(&self) -> u64 {
        self.seq
    }

    /// Every matching raw row, in order.
    pub fn match_rows(&self) -> impl Iterator<Item = usize> + '_ {
        self.matches
            .iter()
            .flat_map(|chunk| chunk.iter().map(|&row| row as usize))
    }

    /// Whether these results answer this query, index and stash-row revision.
    /// A stash list can change the displayed text without replacing the index.
    pub fn is_for(
        &self,
        query: &HistoryFindQuery,
        index: &HistoryIndexHandle,
        stashes_rev: u64,
    ) -> bool {
        self.stashes_rev == stashes_rev
            && self.query.as_ref() == Some(query)
            && self
                .index
                .as_ref()
                .is_some_and(|known| known.ptr_eq(&Arc::downgrade(index)))
    }
}

#[derive(Clone, Debug)]
pub struct HistoryFindChunk {
    /// Newly found raw rows, all greater than any row reported before.
    /// Index rows fit in `u32` (see `HistoryIndexBuilder::push`).
    pub matches: Vec<u32>,
    pub done: bool,
}

#[derive(Debug)]
pub enum HistoryFindMsg {
    /// Start a search, or with `query: None` stop searching and clear results.
    /// Stopping keeps text for the next query in this repo. Searching another
    /// repo releases it, as does `Close`.
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
    /// The find bar closed: stop every repository's search and free the
    /// commit text kept for the next query.
    Close,
}

#[derive(Clone, Debug)]
pub struct HistoryFindEffect {
    pub repo_id: RepoId,
    pub seq: u64,
    pub index: HistoryIndexHandle,
    pub query: HistoryFindQuery,
    /// Listed stashes: their rows show, and are matched on, these messages.
    pub stashes: Arc<Vec<gitcomet_core::domain::StashEntry>>,
    pub cancellation: CancellationToken,
    pub(crate) cache: Arc<Mutex<HistoryFindCache>>,
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
