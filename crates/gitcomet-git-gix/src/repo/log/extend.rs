//! Growing a history index when its history only gained commits on top.
//!
//! Date order shows a commit once all its children are shown, newest first.
//! When every added commit is at least as new as the older index's first row,
//! they all come before it and the older rows follow unchanged: the walk for
//! the larger history reaches the older walk's starting state after them.

use super::index::{HistoryQuery, retain_probable_stashes};
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

/// The commits a query adds to an older index, in the order they were found.
struct Added {
    rows: FxHashMap<gix::ObjectId, u32>,
    ids: Vec<gix::ObjectId>,
    parents: Vec<smallvec::SmallVec<[gix::ObjectId; 2]>>,
    times: Vec<i64>,
    /// Rows of the older index that a new tip or an added commit points at.
    reached: FxHashSet<u32>,
}

fn header(
    objects: &gix::OdbHandleArc,
    id: &gix::oid,
    buffer: &mut Vec<u8>,
) -> Result<(smallvec::SmallVec<[gix::ObjectId; 2]>, i64)> {
    record(Work::IndexObjectRead);
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
    /// `None` past [`ADDED_COMMIT_LIMIT`].
    fn find(
        objects: &gix::OdbHandleArc,
        tips: &[gix::ObjectId],
        older: &HistoryIndex,
        cancellation: &CancellationToken,
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
            if let Some(row) = older.position_bytes(id.as_bytes()) {
                added.reached.insert(row as u32);
                continue;
            }
            if added.ids.len() >= ADDED_COMMIT_LIMIT {
                return Ok(None);
            }
            cancellation.check_cancelled()?;
            let (parents, time) = header(objects, &id, &mut buffer)?;
            added.rows.insert(id, added.ids.len() as u32);
            pending.extend(parents.iter().copied());
            added.ids.push(id);
            added.parents.push(parents);
            added.times.push(time);
        }
        Ok(Some(added))
    }

    /// Whether every commit of the older index is still reached. A tip it was
    /// walked from may be gone only if another of its rows has it as a parent.
    fn keeps(&self, older_tips: &[gix::ObjectId], older: &HistoryIndex) -> bool {
        let mut gone = FxHashSet::default();
        for tip in older_tips {
            match older.position_bytes(tip.as_bytes()) {
                Some(row) if self.reached.contains(&(row as u32)) => {}
                Some(row) => {
                    gone.insert(row as u32);
                }
                None => return false,
            }
        }
        for row in 0..older.len() {
            if gone.is_empty() {
                break;
            }
            for parent in older.parents(row) {
                gone.remove(parent);
            }
        }
        gone.is_empty()
    }

    /// The added commits in date order, or `None` when one of them is older
    /// than `floor`, the time of the older index's first row.
    fn date_order(&self, floor: i64) -> Option<Vec<u32>> {
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
    let Some(added) = Added::find(&objects, &query.tips, older, cancellation)? else {
        return Ok(None);
    };
    if !added.keeps(&older_query.tips, older) {
        return Ok(None);
    }
    let (_, floor) = header(
        &objects,
        gix::oid::from_bytes_unchecked(first),
        &mut Vec::new(),
    )?;
    let Some(order) = added.date_order(floor) else {
        return Ok(None);
    };

    let repo = store.to_thread_local();
    let mut builder =
        HistoryIndexBuilder::new(snapshot, query.mode, repo.object_hash().len_in_bytes())?;
    for row in order {
        let parents = &added.parents[row as usize];
        builder.push(
            added.ids[row as usize].as_bytes(),
            parents.iter().map(|id| id.as_bytes()),
            (2..=3).contains(&parents.len()),
        )?;
    }
    retain_probable_stashes(&mut builder, &repo, false, cancellation)?;
    let index = builder.finish_before(older, cancellation)?;
    record(Work::HistoryIndexExtend);
    Ok(Some(index))
}
