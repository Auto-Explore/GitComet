use super::*;
use crate::indexed_history::{
    HistoryReadPriority, HistoryViewport, IndexedHistoryEffect as Work, IndexedHistoryMsg as Event,
    IndexedHistoryState, PendingRange,
};
use gitcomet_core::history_index::{HISTORY_BLOCK_SIZE, HISTORY_ROW_CACHE_LIMIT};
use gitcomet_core::services::CancellationToken;

pub(super) fn reduce(state: &mut AppState, event: Event) -> Vec<Effect> {
    let repo_id = match &event {
        Event::Retry { repo_id }
        | Event::Select { repo_id, .. }
        | Event::Publish { repo_id, .. }
        | Event::Ensure { repo_id }
        | Event::RequestRanges { repo_id, .. }
        | Event::RequestViewport { repo_id, .. }
        | Event::Progress { repo_id, .. }
        | Event::Built { repo_id, .. }
        | Event::RangeLoaded { repo_id, .. } => *repo_id,
    };
    let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) else {
        return Vec::new();
    };
    match event {
        Event::Retry { .. } => {
            repo.history_state.indexed.requested = None;
            return reduce(state, Event::Ensure { repo_id });
        }
        Event::Select {
            commit_id,
            mode,
            projection,
            ..
        } => {
            let clicked = repo
                .history_state
                .indexed
                .displayed_index
                .as_ref()
                .is_some_and(|index| Arc::ptr_eq(index, &projection.index))
                .then(|| projection.position(commit_id.as_ref()))
                .flatten();
            let Some(clicked) = clicked else {
                return Vec::new();
            };
            let entries = if mode == crate::msg::CommitSelectMode::Range {
                let anchor = repo
                    .history_state
                    .multi_selection
                    .anchor
                    .as_ref()
                    .and_then(|id| projection.position(id.as_ref()))
                    .unwrap_or(clicked);
                Some(
                    (anchor.min(clicked)..=anchor.max(clicked))
                        .filter_map(|row| projection.commit_id(row))
                        .collect(),
                )
            } else {
                None
            };
            // Only materialize the selected range, never every ID in history.
            return effects::select_commit_multi(state, repo_id, commit_id, mode, None, entries);
        }
        Event::Publish { index, .. } => {
            let history = &mut repo.history_state.indexed;
            // The store can finish a newer build before it processes the UI's
            // publication. The prepared range source is still a valid display.
            if !history
                .index
                .iter()
                .chain(history.range_index.iter())
                .any(|known| Arc::ptr_eq(known, &index))
                || history
                    .displayed_index
                    .as_ref()
                    .is_some_and(|shown| Arc::ptr_eq(shown, &index))
            {
                return Vec::new();
            }
            history.displayed_index = Some(index);
            history.rev = history.rev.wrapping_add(1);
            return Vec::new();
        }
        Event::Ensure { .. } => {
            let Some(snapshot) = repo.history_state.log_snapshot.clone() else {
                return Vec::new();
            };
            let walk = repo.history_walk();
            let history = &mut repo.history_state.indexed;
            if history.requested.as_ref() == Some(&snapshot) && history.epoch == repo.load_epoch {
                return Vec::new();
            }
            if history
                .index
                .as_ref()
                .is_some_and(|index| index.snapshot == snapshot)
            {
                history.epoch = repo.load_epoch;
                history.requested = Some(snapshot);
                history.loading = false;
                history.progress = None;
                history.rev = history.rev.wrapping_add(1);
                history.status_rev = history.status_rev.wrapping_add(1);
            } else {
                history.cancellation.cancel();
                history.seq = history.seq.wrapping_add(1);
                history.epoch = repo.load_epoch;
                history.requested = Some(snapshot);
                history.loading = true;
                history.progress = None;
                history.error = None;
                history.cancellation = CancellationToken::new();
                history.rev = history.rev.wrapping_add(1);
                history.status_rev = history.status_rev.wrapping_add(1);
                return vec![Effect::IndexedHistory(Work::Build {
                    repo_id,
                    seq: history.seq,
                    walk,
                    author: repo.history_state.history_author_filter.clone(),
                    focus: repo.history_state.history_focus.clone(),
                    cancellation: history.cancellation.clone(),
                })];
            }
        }
        Event::Progress { seq, progress, .. } => {
            let history = &mut repo.history_state.indexed;
            if seq == history.seq && history.loading && history.epoch == repo.load_epoch {
                history.progress = Some(progress);
                history.status_rev = history.status_rev.wrapping_add(1);
            }
        }
        Event::Built { seq, result, .. } => {
            let history = &mut repo.history_state.indexed;
            if seq != history.seq || history.epoch != repo.load_epoch {
                return Vec::new();
            }
            history.loading = false;
            history.progress = None;
            match result {
                Ok(Some(index))
                    if Some(&index.snapshot) == history.requested.as_ref()
                        && Some(&index.snapshot) == repo.history_state.log_snapshot.as_ref() =>
                {
                    let mut index = index;
                    // The backend formats its own snapshot text. Sharing the
                    // log's allocation turns every later equality check into a
                    // pointer comparison instead of a memcmp of every tip.
                    if let (Some(unique), Some(snapshot)) = (
                        Arc::get_mut(&mut index),
                        repo.history_state.log_snapshot.as_ref(),
                    ) {
                        unique.snapshot = snapshot.clone();
                    }
                    history.index = Some(index.clone());
                    history.rev = history.rev.wrapping_add(1);
                    history.status_rev = history.status_rev.wrapping_add(1);
                    let Some(pruned) = effects::prune_multi_selection(repo, |id| {
                        index.position(id.as_ref()).is_some()
                    }) else {
                        return Vec::new();
                    };
                    return effects::apply_pruned_selection(state, repo_id, pruned, Some(&index));
                }
                Ok(Some(_)) => {
                    history.requested = None;
                }
                Ok(None) => {}
                Err(error) => {
                    history.error = Some(error.to_string());
                }
            }
            history.rev = history.rev.wrapping_add(1);
            history.status_rev = history.status_rev.wrapping_add(1);
        }
        Event::RequestRanges {
            snapshot,
            blocks,
            retry,
            ..
        } => {
            let viewport = HistoryViewport {
                visible_count: blocks.len(),
                blocks,
            };
            update_viewport(&mut repo.history_state.indexed, snapshot, viewport, retry);
        }
        Event::RequestViewport {
            snapshot,
            viewport,
            retry,
            ..
        } => {
            update_viewport(&mut repo.history_state.indexed, snapshot, viewport, retry);
        }
        Event::RangeLoaded {
            snapshot,
            seq,
            start,
            result,
            ..
        } => {
            let history = &mut repo.history_state.indexed;
            if history
                .range_index
                .as_ref()
                .is_none_or(|index| index.snapshot != snapshot)
                || history
                    .pending
                    .get(&start)
                    .is_none_or(|pending| pending.seq != seq)
            {
                return Vec::new();
            }
            history.pending.remove(&start);
            match result {
                Ok(range)
                    if range.snapshot == snapshot
                        && range.start == start
                        && history.range_index.as_ref().is_some_and(|index| {
                            range.commits.len()
                                == HISTORY_BLOCK_SIZE.min(index.len().saturating_sub(start))
                                && range.commits.iter().enumerate().all(|(ix, commit)| {
                                    index.row_matches_hex_id(start + ix, commit.id.as_ref())
                                })
                        }) =>
                {
                    history.lru.retain(|entry| *entry != start);
                    history.lru.push_back(start);
                    history.ranges.insert(start, range.clone());
                    history.ranges_rev = history.ranges_rev.wrapping_add(1);
                    while history.ranges.len() > HISTORY_ROW_CACHE_LIMIT / HISTORY_BLOCK_SIZE {
                        let Some(oldest) = history.lru.pop_front() else {
                            break;
                        };
                        if history.desired.blocks.contains(&oldest) {
                            history.lru.push_back(oldest);
                        } else {
                            history.ranges.remove(&oldest);
                        }
                    }
                }
                Ok(_) => {
                    history
                        .range_errors
                        .insert(start, "History response did not match its request".into());
                }
                Err(error) => {
                    history.range_errors.insert(start, error.to_string());
                }
            }
            history.rev = history.rev.wrapping_add(1);
        }
    }
    schedule_ranges(&mut repo.history_state.indexed, repo_id)
}

fn update_viewport(
    history: &mut IndexedHistoryState,
    snapshot: gitcomet_core::services::HistorySnapshot,
    viewport: HistoryViewport,
    retry: bool,
) {
    let index = history
        .index
        .as_ref()
        .filter(|index| index.snapshot == snapshot)
        .or_else(|| {
            history
                .range_index
                .as_ref()
                .filter(|index| index.snapshot == snapshot)
        })
        .cloned();
    let Some(index) = index else {
        return;
    };
    if history
        .range_index
        .as_ref()
        .is_none_or(|old| old.snapshot != snapshot)
    {
        for pending in history.pending.values() {
            pending.cancellation.cancel();
        }
        history.pending.clear();
        history.ranges.clear();
        history.ranges_rev = history.ranges_rev.wrapping_add(1);
        history.range_errors.clear();
        history.lru.clear();
        history.range_index = Some(index.clone());
        history.rev = history.rev.wrapping_add(1);
    }
    let mut desired = HistoryViewport::default();
    for (position, block) in viewport
        .blocks
        .into_iter()
        .take(HISTORY_ROW_CACHE_LIMIT / HISTORY_BLOCK_SIZE)
        .enumerate()
    {
        if block < index.len()
            && block.is_multiple_of(HISTORY_BLOCK_SIZE)
            && !desired.blocks.contains(&block)
        {
            desired.blocks.push(block);
            if position < viewport.visible_count {
                desired.visible_count += 1;
            }
        }
    }
    history
        .range_errors
        .retain(|start, _| desired.blocks.contains(start));
    if retry {
        for start in &desired.blocks {
            history.range_errors.remove(start);
        }
    }
    for start in &desired.blocks {
        if history.ranges.contains_key(start) {
            history.lru.retain(|entry| entry != start);
            history.lru.push_back(*start);
        }
    }
    history.desired = desired;
}

fn schedule_ranges(history: &mut IndexedHistoryState, repo_id: RepoId) -> Vec<Effect> {
    let Some(index) = history.range_index.clone() else {
        return Vec::new();
    };
    let next: smallvec::SmallVec<[_; 2]> = history
        .desired
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, start)| {
            !history.ranges.contains_key(*start) && !history.range_errors.contains_key(*start)
        })
        .take(2)
        .map(|(position, &start)| {
            let priority = if position < history.desired.visible_count {
                HistoryReadPriority::Visible
            } else {
                HistoryReadPriority::Prefetch
            };
            (start, priority)
        })
        .collect();
    history.pending.retain(|start, pending| {
        let keep = next.contains(&(*start, pending.priority));
        if !keep {
            pending.cancellation.cancel();
        }
        keep
    });
    let mut effects = Vec::new();
    for (start, priority) in next {
        if history.pending.contains_key(&start) {
            continue;
        }
        history.range_seq = history.range_seq.wrapping_add(1);
        let cancellation = CancellationToken::new();
        history.pending.insert(
            start,
            PendingRange {
                seq: history.range_seq,
                cancellation: cancellation.clone(),
                priority,
            },
        );
        effects.push(Effect::IndexedHistory(Work::Range {
            repo_id,
            seq: history.range_seq,
            index: index.clone(),
            start,
            priority,
            cancellation,
        }));
    }
    effects
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod snapshot_regressions;
