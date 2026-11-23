//! Growing a date-ordered history when it only gained commits on top.
//!
//! Date order shows a commit once all its children are shown, newest first.
//! When every added commit is at least as new as the older order's first row,
//! they all come before it and the older rows follow unchanged: the walk for
//! the larger history reaches the older walk's starting state after them.
//! Both the history index and the header topology grow this way.

use super::index::{HistoryQuery, StashShapes, retain_probable_stashes};
use super::walk::log_paged_walk_handle;
use crate::repo::ShallowSnapshot;
use gitcomet_core::domain::HistoryMode;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::history_index::{HistoryIndex, HistoryIndexBuilder, HistoryIndexHandle};
use gitcomet_core::history_perf::{Work, record};
use gitcomet_core::services::{CancellationToken, HistorySnapshot, Result};
use gix::objs::FindExt as _;
use rustc_hash::{FxHashMap, FxHashSet};

impl HistoryQuery {
    /// Whether an index for `older` can be grown into this query's: the same
    /// walk of every commit over the same store, from other tips.
    pub(in crate::repo) fn extends(&self, older: &Self) -> bool {
        let shallow: &ShallowSnapshot = &self.shallow;
        self.tips != older.tips
            && self.common == older.common
            && self.generation == older.generation
            && self.mode == older.mode
            && matches!(
                self.mode,
                HistoryMode::FullReachable | HistoryMode::AllBranches
            )
            && self.author.is_none()
            && older.author.is_none()
            && self.focus == older.focus
            && self.shallow == older.shallow
            && !shallow.is_shallow()
    }
}

/// Added commits are read one header at a time, about 50 µs each without a
/// commit-graph. Past this many a rebuild, which walks the graph, is the
/// faster way to the same index.
const ADDED_COMMIT_LIMIT: usize = 20_000;

/// The commits new tips add to an older history, in the order they were found.
pub(super) struct Added {
    pub(super) rows: FxHashMap<gix::ObjectId, u32>,
    pub(super) ids: Vec<gix::ObjectId>,
    pub(super) parents: Vec<smallvec::SmallVec<[gix::ObjectId; 2]>>,
    pub(super) times: Vec<i64>,
    /// Rows of the older history that a new tip or an added commit points at.
    reached: FxHashSet<u32>,
}

fn header(
    objects: &gix::OdbHandleArc,
    id: &gix::oid,
    buffer: &mut Vec<u8>,
    work: Work,
) -> Result<(smallvec::SmallVec<[gix::ObjectId; 2]>, i64)> {
    record(work);
    let failed = |error: &dyn std::fmt::Display| {
        Error::new(ErrorKind::Backend(format!("gix history index: {error}")))
    };
    let mut parents = smallvec::SmallVec::new();
    for token in objects
        .find_commit_iter(id, buffer)
        .map_err(|error| failed(&error))?
    {
        use gix::objs::commit::ref_iter::Token;
        match token.map_err(|error| failed(&error))? {
            Token::Parent { id } => parents.push(id),
            Token::Committer { signature } => return Ok((parents, signature.seconds())),
            Token::Tree { .. } | Token::Author { .. } => {}
            _ => break,
        }
    }
    Ok((parents, 0))
}

impl Added {
    /// Walks from `tips` to the rows `row_of` knows, reading each added
    /// commit's header as `work`. `None` past [`ADDED_COMMIT_LIMIT`].
    pub(super) fn find(
        objects: &gix::OdbHandleArc,
        tips: &[gix::ObjectId],
        row_of: &dyn Fn(&gix::oid) -> Option<u32>,
        work: Work,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Self>> {
        let mut added = Self {
            rows: FxHashMap::default(),
            ids: Vec::new(),
            parents: Vec::new(),
            times: Vec::new(),
            reached: FxHashSet::default(),
        };
        let mut buffer = Vec::new();
        let mut pending = tips.to_vec();
        while let Some(id) = pending.pop() {
            if added.rows.contains_key(&id) {
                continue;
            }
            if let Some(row) = row_of(&id) {
                added.reached.insert(row);
                continue;
            }
            if added.ids.len() >= ADDED_COMMIT_LIMIT {
                return Ok(None);
            }
            if cancelled() {
                return Err(Error::new(ErrorKind::Cancelled));
            }
            let (parents, time) = header(objects, &id, &mut buffer, work)?;
            added.rows.insert(id, added.ids.len() as u32);
            pending.extend(parents.iter().copied());
            added.ids.push(id);
            added.parents.push(parents);
            added.times.push(time);
        }
        Ok(Some(added))
    }

    /// Whether every commit of the older history is still reached. A tip it
    /// was walked from may be gone only if another of its rows has it as a
    /// parent; `parent_rows` lists the parent of every edge between its rows.
    pub(super) fn keeps(
        &self,
        older_tips: &[gix::ObjectId],
        row_of: &dyn Fn(&gix::oid) -> Option<u32>,
        parent_rows: impl IntoIterator<Item = u32>,
    ) -> bool {
        let mut gone = FxHashSet::default();
        for tip in older_tips {
            match row_of(tip) {
                Some(row) if self.reached.contains(&row) => {}
                Some(row) => {
                    gone.insert(row);
                }
                None => return false,
            }
        }
        if gone.is_empty() {
            return true;
        }
        for parent in parent_rows {
            if gone.remove(&parent) && gone.is_empty() {
                return true;
            }
        }
        false
    }

    /// The added commits in date order, or `None` when one of them is older
    /// than `floor`, the time of the older index's first row.
    pub(super) fn date_order(&self, floor: i64) -> Option<Vec<u32>> {
        let mut children = vec![0u32; self.ids.len()];
        for parent in self.parents.iter().flatten() {
            if let Some(&row) = self.rows.get(parent) {
                children[row as usize] += 1;
            }
        }
        // The queue the full walk uses, so equal times fall the same way.
        let mut ready = gix::revwalk::PriorityQueue::new();
        for (row, &count) in children.iter().enumerate() {
            if count == 0 {
                ready.insert(self.times[row], row as u32);
            }
        }
        let mut order = Vec::with_capacity(self.ids.len());
        while let Some(row) = ready.pop_value() {
            if self.times[row as usize] < floor {
                return None;
            }
            order.push(row);
            for parent in &self.parents[row as usize] {
                if let Some(&parent) = self.rows.get(parent) {
                    children[parent as usize] -= 1;
                    if children[parent as usize] == 0 {
                        ready.insert(self.times[parent as usize], parent);
                    }
                }
            }
        }
        (order.len() == self.ids.len()).then_some(order)
    }
}

/// The index for `query`: `older`, built for `older_query`, with the commits
/// `query` adds in front. `None` when a rebuild could differ: `older` holds
/// commits `query` no longer reaches, or an added commit is dated before
/// `older`'s first row and belongs among its rows.
pub(in crate::repo) fn extend_history_index(
    store: &gix::ThreadSafeRepository,
    query: &HistoryQuery,
    snapshot: HistorySnapshot,
    older_query: &HistoryQuery,
    older: &HistoryIndex,
    cancellation: &CancellationToken,
) -> Result<Option<HistoryIndexHandle>> {
    cancellation.check_cancelled()?;
    let Some(first) = older.id_bytes(0) else {
        return Ok(None);
    };
    if !query.extends(older_query) {
        return Ok(None);
    }
    let objects = log_paged_walk_handle(store);
    let row_of = |id: &gix::oid| older.position_bytes(id.as_bytes()).map(|row| row as u32);
    let Some(added) = Added::find(
        &objects,
        &query.tips,
        &row_of,
        Work::IndexObjectRead,
        &|| cancellation.is_cancelled(),
    )?
    else {
        return Ok(None);
    };
    let parent_rows = (0..older.len()).flat_map(|row| older.parents(row).iter().copied());
    if !added.keeps(&older_query.tips, &row_of, parent_rows) {
        return Ok(None);
    }
    let (_, floor) = header(
        &objects,
        gix::oid::from_bytes_unchecked(first),
        &mut Vec::new(),
        Work::IndexObjectRead,
    )?;
    let Some(order) = added.date_order(floor) else {
        return Ok(None);
    };

    let repo = store.to_thread_local();
    let mut builder =
        HistoryIndexBuilder::new(snapshot, query.mode, repo.object_hash().len_in_bytes())?;
    let mut shapes = StashShapes::default();
    for row in order {
        let id = added.ids[row as usize];
        let parents = &added.parents[row as usize];
        shapes.visit(id, parents);
        builder.push(
            id.as_bytes(),
            parents.iter().map(|id| id.as_bytes()),
            (2..=3).contains(&parents.len()),
        )?;
    }
    // A merge on top can name older rows as its other parents, which a
    // rebuild passes and this walk does not.
    for id in shapes.awaited() {
        let Some(row) = older.position_bytes(id.as_bytes()) else {
            continue;
        };
        let parents: Option<Vec<gix::ObjectId>> = (0..older.parents(row).len())
            .map(|parent| {
                let id = older.parent_id_bytes(row, parent)?;
                Some(gix::oid::from_bytes_unchecked(id).to_owned())
            })
            .collect();
        shapes.passed(id, parents.as_deref());
    }
    retain_probable_stashes(&mut builder, &repo, &shapes, false, cancellation)?;
    let index = builder.finish_before(older, cancellation)?;
    record(Work::HistoryIndexExtend);
    Ok(Some(index))
}
