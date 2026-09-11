use super::*;
use crate::indexed_history::{IndexedHistoryEffect as Work, IndexedHistoryMsg as Event};
use gitcomet_core::history_index::{HISTORY_BLOCK_SIZE, HISTORY_ROW_CACHE_LIMIT};
use gitcomet_core::services::CancellationToken;

pub(super) fn reduce(state: &mut AppState, event: Event) -> Vec<Effect> {
    let repo_id = match &event {
        Event::Retry { repo_id }
        | Event::Select { repo_id, .. }
        | Event::Ensure { repo_id }
        | Event::RequestRanges { repo_id, .. }
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
            if repo
                .history_state
                .indexed
                .range_index
                .as_ref()
                .is_none_or(|index| !Arc::ptr_eq(index, &projection.index))
            {
                return Vec::new();
            }
            let Some(clicked) = projection.position(commit_id.as_ref()) else {
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
        Event::Ensure { .. } => {
            let Some(snapshot) = repo.history_state.log_snapshot.clone() else {
                return Vec::new();
            };
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
                return vec![Effect::IndexedHistory(Work::Build {
                    repo_id,
                    seq: history.seq,
                    mode: repo.history_state.history_scope,
                    author: repo.history_state.history_author_filter.clone(),
                    cancellation: history.cancellation.clone(),
                })];
            }
        }
        Event::Progress { seq, progress, .. } => {
            let history = &mut repo.history_state.indexed;
            if seq == history.seq && history.loading && history.epoch == repo.load_epoch {
                history.progress = Some(progress);
                history.rev = history.rev.wrapping_add(1);
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
                    history.index = Some(index.clone());
                    history.rev = history.rev.wrapping_add(1);
                    let reveal = repo.history_state.reveal_target.clone();
                    let survives = |id: &gitcomet_core::domain::CommitId| {
                        reveal.as_ref() == Some(id) || index.position(id.as_ref()).is_some()
                    };
                    let mut selection = repo.history_state.multi_selection.clone();
                    selection.commits.retain(&survives);
                    if selection.anchor.as_ref().is_some_and(|id| !survives(id)) {
                        selection.anchor = None;
                    }
                    selection.anchor_index = None;
                    selection.anchor_log_rev = None;
                    let missing_focus = repo
                        .history_state
                        .selected_commit
                        .as_ref()
                        .is_some_and(|id| !survives(id));
                    let fallback = selection.commits.last().cloned();
                    repo.set_commit_multi_selection(selection);
                    if missing_focus {
                        if let Some(id) = fallback {
                            return effects::select_commit_and_load_details(repo, repo_id, id);
                        }
                        repo.set_selected_commit(None);
                        repo.set_commit_details(Loadable::NotLoaded);
                    }
                    return Vec::new();
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
        }
        Event::RequestRanges {
            snapshot,
            blocks,
            retry,
            ..
        } => {
            let history = &mut repo.history_state.indexed;
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
                return Vec::new();
            };
            if history
                .range_index
                .as_ref()
                .is_none_or(|old| old.snapshot != snapshot)
            {
                for (_, token) in history.pending.values() {
                    token.cancel();
                }
                history.pending.clear();
                history.ranges.clear();
                history.range_errors.clear();
                history.lru.clear();
                history.range_index = Some(index.clone());
            }
            let mut desired = Vec::new();
            for block in blocks
                .into_iter()
                .take(HISTORY_ROW_CACHE_LIMIT / HISTORY_BLOCK_SIZE)
            {
                if block < index.len()
                    && block.is_multiple_of(HISTORY_BLOCK_SIZE)
                    && !desired.contains(&block)
                {
                    desired.push(block);
                }
            }
            history.pending.retain(|start, (_, token)| {
                if desired.contains(start) {
                    true
                } else {
                    token.cancel();
                    false
                }
            });
            history
                .range_errors
                .retain(|start, _| desired.contains(start));
            if retry {
                for start in &desired {
                    history.range_errors.remove(start);
                }
            }
            for start in &desired {
                if history.ranges.contains_key(start) {
                    history.lru.retain(|entry| entry != start);
                    history.lru.push_back(*start);
                }
            }
            history.desired = desired;
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
                    .is_none_or(|(pending, _)| *pending != seq)
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
                                    index.position(commit.id.as_ref()) == Some(start + ix)
                                })
                        }) =>
                {
                    history.lru.retain(|entry| *entry != start);
                    history.lru.push_back(start);
                    history.ranges.insert(start, Arc::new(range));
                    while history.ranges.len() > HISTORY_ROW_CACHE_LIMIT / HISTORY_BLOCK_SIZE {
                        let Some(oldest) = history.lru.pop_front() else {
                            break;
                        };
                        if history.desired.contains(&oldest) {
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
    let history = &mut repo.history_state.indexed;
    let Some(index) = history.range_index.clone() else {
        return Vec::new();
    };
    let mut effects = Vec::new();
    for &start in &history.desired {
        if history.pending.len() >= 2 {
            break;
        }
        if history.pending.contains_key(&start)
            || history.ranges.contains_key(&start)
            || history.range_errors.contains_key(&start)
        {
            continue;
        }
        history.range_seq = history.range_seq.wrapping_add(1);
        let cancellation = CancellationToken::new();
        history
            .pending
            .insert(start, (history.range_seq, cancellation.clone()));
        effects.push(Effect::IndexedHistory(Work::Range {
            repo_id,
            seq: history.range_seq,
            index: index.clone(),
            start,
            cancellation,
        }));
    }
    effects
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RepoState;
    use crate::msg::CommitSelectMode;
    use gitcomet_core::domain::{Commit, LogScope, RepoSpec};
    use gitcomet_core::history_index::{
        HistoryIndexBuilder, HistoryIndexHandle, HistoryProjection, HistoryRange,
    };
    use gitcomet_core::services::HistorySnapshot;

    fn fixture() -> (AppState, HistoryIndexHandle) {
        let mut builder =
            HistoryIndexBuilder::new(HistorySnapshot("index".into()), LogScope::AllBranches, 20)
                .unwrap();
        for row in 0..12_000u32 {
            let mut id = [0u8; 20];
            id[..4].copy_from_slice(&row.to_be_bytes());
            let mut parent = [0u8; 20];
            parent[..4].copy_from_slice(&(row + 1).to_be_bytes());
            builder.push(&id, [parent.as_slice()], 0, false).unwrap();
        }
        let index = builder.finish(&CancellationToken::new()).unwrap();
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: "/tmp/indexed-history-test".into(),
            },
        );
        repo.history_state.indexed.index = Some(index.clone());
        repo.history_state.log_snapshot = Some(index.snapshot.clone());
        (
            AppState {
                repos: vec![repo],
                active_repo: Some(RepoId(1)),
                ..Default::default()
            },
            index,
        )
    }
    fn request(
        state: &mut AppState,
        index: &HistoryIndexHandle,
        blocks: Vec<usize>,
    ) -> Vec<Effect> {
        reduce(
            state,
            Event::RequestRanges {
                repo_id: RepoId(1),
                snapshot: index.snapshot.clone(),
                blocks,
                retry: false,
            },
        )
    }
    fn loaded(work: Effect) -> Event {
        let Effect::IndexedHistory(Work::Range {
            repo_id,
            seq,
            index,
            start,
            ..
        }) = work
        else {
            panic!("expected range");
        };
        Event::RangeLoaded {
            repo_id,
            seq,
            snapshot: index.snapshot.clone(),
            start,
            result: Ok(HistoryRange {
                snapshot: index.snapshot.clone(),
                start,
                commits: (start..(start + 256).min(index.len()))
                    .map(|row| Commit {
                        id: index.commit_id(row).unwrap(),
                        parent_ids: index.parent_commit_id(row, 0).into_iter().collect(),
                        author: "author".into(),
                        summary: "summary".into(),
                        time: std::time::UNIX_EPOCH,
                    })
                    .collect(),
            }),
        }
    }
    #[test]
    fn distant_requests_cancel_old_work_and_ignore_late_responses() {
        let (mut state, index) = fixture();
        let mut first = request(&mut state, &index, vec![0, 256, 512]);
        assert_eq!(first.len(), 2);
        let token = match &first[0] {
            Effect::IndexedHistory(Work::Range { cancellation, .. }) => cancellation.clone(),
            _ => unreachable!(),
        };
        let next = request(&mut state, &index, vec![10_240, 10_496]);
        assert_eq!(next.len(), 2);
        assert!(token.is_cancelled());
        assert!(reduce(&mut state, loaded(first.remove(0))).is_empty());
        assert!(state.repos[0].history_state.indexed.ranges.is_empty());
        for work in next {
            reduce(&mut state, loaded(work));
        }
        assert_eq!(
            state.repos[0]
                .history_state
                .indexed
                .ranges
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            [10_240, 10_496]
        );
    }
    #[test]
    fn decoded_cache_is_bounded_and_bad_ranges_require_retry() {
        let (mut state, index) = fixture();
        for block in (0..11_776).step_by(256) {
            for work in request(&mut state, &index, vec![block]) {
                reduce(&mut state, loaded(work));
            }
            assert!(
                state.repos[0].history_state.indexed.ranges.len()
                    <= HISTORY_ROW_CACHE_LIMIT / HISTORY_BLOCK_SIZE
            );
        }
        assert!(!state.repos[0].history_state.indexed.ranges.contains_key(&0));
        let work = request(&mut state, &index, vec![0]).remove(0);
        let mut event = loaded(work);
        if let Event::RangeLoaded {
            result: Ok(range), ..
        } = &mut event
        {
            range.commits.pop();
        }
        reduce(&mut state, event);
        assert!(
            state.repos[0]
                .history_state
                .indexed
                .range_errors
                .contains_key(&0)
        );
        assert!(request(&mut state, &index, vec![0]).is_empty());
        assert_eq!(
            reduce(
                &mut state,
                Event::RequestRanges {
                    repo_id: RepoId(1),
                    snapshot: index.snapshot.clone(),
                    blocks: vec![0],
                    retry: true
                }
            )
            .len(),
            1
        );
    }
    #[test]
    fn shift_selection_and_comparison_resolve_outside_the_bootstrap_page() {
        let (mut state, index) = fixture();
        request(&mut state, &index, vec![9984]);
        let projection = HistoryProjection::new(index.clone(), vec![10_002]);
        for (row, mode) in [
            (10_000, CommitSelectMode::Single),
            (10_004, CommitSelectMode::Range),
        ] {
            reduce(
                &mut state,
                Event::Select {
                    repo_id: RepoId(1),
                    projection: projection.clone(),
                    commit_id: index.commit_id(row).unwrap(),
                    mode,
                },
            );
        }
        assert_eq!(
            state.repos[0].history_state.multi_selection.commits,
            [10_000, 10_001, 10_003, 10_004].map(|row| index.commit_id(row).unwrap())
        );
        assert!(state.repos[0].history_state.multi_selection.is_multi());
    }
    #[test]
    fn query_change_invalidates_generations_even_when_switching_back() {
        let (mut state, _) = fixture();
        state.repos[0].history_state.indexed.index = None;
        let work = reduce(&mut state, Event::Ensure { repo_id: RepoId(1) }).remove(0);
        let Effect::IndexedHistory(Work::Build {
            seq, cancellation, ..
        }) = work
        else {
            panic!()
        };
        state.repos[0].set_log_scope(LogScope::FirstParent);
        state.repos[0].set_log_scope(LogScope::AllBranches);
        assert!(cancellation.is_cancelled());
        reduce(&mut state, Event::Ensure { repo_id: RepoId(1) });
        let rev = state.repos[0].history_state.indexed.rev;
        reduce(
            &mut state,
            Event::Built {
                repo_id: RepoId(1),
                seq,
                result: Ok(None),
            },
        );
        assert_eq!(state.repos[0].history_state.indexed.rev, rev);
        assert!(state.repos[0].history_state.indexed.loading);
    }
}
