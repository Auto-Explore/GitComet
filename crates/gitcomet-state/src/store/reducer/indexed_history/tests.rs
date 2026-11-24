use super::*;
use crate::model::RepoState;
use crate::msg::CommitSelectMode;
use gitcomet_core::domain::{Commit, LogScope, RepoSpec};
use gitcomet_core::history_index::{
    HistoryIndexBuilder, HistoryIndexHandle, HistoryIndexProgress, HistoryProjection, HistoryRange,
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
        builder.push(&id, [parent.as_slice()], false).unwrap();
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
            git_log_settings: crate::model::GitLogSettings {
                verify_commit_signatures: false,
                ..Default::default()
            },
            ..AppState::test_default()
        },
        index,
    )
}
fn request(state: &mut AppState, index: &HistoryIndexHandle, blocks: Vec<usize>) -> Vec<Effect> {
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
        result: Ok(Arc::new(HistoryRange {
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
        })),
    }
}

#[test]
fn delivered_range_keeps_the_backend_allocation() {
    let (mut state, index) = fixture();
    let work = request(&mut state, &index, vec![0]).remove(0);
    let event = loaded(work);
    let Event::RangeLoaded {
        result: Ok(block), ..
    } = &event
    else {
        panic!()
    };
    let block = block.clone();
    reduce(&mut state, event);
    assert!(Arc::ptr_eq(
        &block,
        &state.repos[0].history_state.indexed.ranges[&0]
    ));
}

#[test]
fn built_index_shares_the_log_snapshot_allocation() {
    let (mut state, _) = fixture();
    let repo = &mut state.repos[0];
    repo.history_state.indexed = Default::default();
    let log_snapshot = repo.history_state.log_snapshot.clone().unwrap();
    let work = reduce(&mut state, Event::Ensure { repo_id: RepoId(1) });
    let Some(Effect::IndexedHistory(Work::Build { seq, .. })) = work.first() else {
        panic!("expected a build");
    };
    // Equal text in a separately allocated snapshot, as the backend produces.
    let mut builder = HistoryIndexBuilder::new(
        HistorySnapshot(Arc::from(log_snapshot.0.as_ref())),
        LogScope::AllBranches,
        20,
    )
    .unwrap();
    builder.push(&[7; 20], std::iter::empty(), false).unwrap();
    let built = builder.finish(&CancellationToken::new()).unwrap();
    assert!(!Arc::ptr_eq(&built.snapshot.0, &log_snapshot.0));
    reduce(
        &mut state,
        Event::Built {
            repo_id: RepoId(1),
            seq: *seq,
            result: Ok(Some(built)),
        },
    );
    let stored = state.repos[0].history_state.indexed.index.clone().unwrap();
    assert!(
        Arc::ptr_eq(&stored.snapshot.0, &log_snapshot.0),
        "the stored index must share the log snapshot's allocation"
    );
    assert_eq!(stored.len(), 1);
}

#[test]
fn prefetched_indexed_ranges_never_schedule_signature_verification() {
    let (mut state, index) = fixture();
    state.git_log_settings.verify_commit_signatures = true;
    state.signing_tools.gpg.availability =
        gitcomet_core::signing_tools::SigningToolAvailability::Unknown;
    let mut work = request(&mut state, &index, vec![9984, 10240, 10496]);
    let next = reduce(&mut state, loaded(work.remove(0)));
    assert!(
        !next
            .iter()
            .any(|effect| matches!(effect, Effect::VerifyCommitSignatures { .. }))
    );
    assert!(
        state.repos[0]
            .history_state
            .commit_signatures_requested
            .is_empty()
    );
    assert_eq!(state.repos[0].history_state.indexed.pending.len(), 2);
    assert!(next.iter().any(|effect| matches!(
        effect,
        Effect::IndexedHistory(Work::Range { start: 10496, .. })
    )));
}

#[test]
fn indexed_stale_and_malformed_ranges_do_not_queue_signature_verification() {
    let (mut state, index) = fixture();
    state.git_log_settings.verify_commit_signatures = true;
    let stale = request(&mut state, &index, vec![0]).remove(0);
    let current = request(&mut state, &index, vec![9984]).remove(0);
    assert!(reduce(&mut state, loaded(stale)).is_empty());
    let mut malformed = loaded(current);
    if let Event::RangeLoaded {
        result: Ok(range), ..
    } = &mut malformed
    {
        Arc::make_mut(range).commits[0].id = index.commit_id(0).unwrap();
    }
    assert!(reduce(&mut state, malformed).is_empty());
    assert!(
        state.repos[0]
            .history_state
            .commit_signatures_requested
            .is_empty()
    );
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
fn visible_blocks_preempt_pending_prefetch_even_when_the_blocks_stay_in_view() {
    use HistoryReadPriority::{Prefetch, Visible};
    let (mut state, index) = fixture();
    let viewport = |blocks: Vec<usize>, visible_count| Event::RequestViewport {
        repo_id: RepoId(1),
        snapshot: index.snapshot.clone(),
        viewport: HistoryViewport {
            blocks,
            visible_count,
        },
        retry: false,
    };
    let requested = |effects: &[Effect]| {
        effects
            .iter()
            .map(|work| match work {
                Effect::IndexedHistory(Work::Range {
                    start, priority, ..
                }) => (*start, *priority),
                _ => panic!("expected range"),
            })
            .collect::<Vec<_>>()
    };
    let first = reduce(&mut state, viewport(vec![0, 256, 512], 1));
    assert_eq!(requested(&first), [(0, Visible), (256, Prefetch)]);
    let pending = state.repos[0].history_state.indexed.pending.clone();
    let moved = reduce(&mut state, viewport(vec![512, 256, 0], 2));
    assert_eq!(requested(&moved), [(512, Visible), (256, Visible)]);
    assert!(
        pending
            .values()
            .all(|work| work.cancellation.is_cancelled())
    );
    for work in first {
        assert!(reduce(&mut state, loaded(work)).is_empty());
    }
    assert!(state.repos[0].history_state.indexed.ranges.is_empty());
    let mut moved = moved.into_iter();
    assert_eq!(
        requested(&reduce(&mut state, loaded(moved.next().unwrap()))),
        [(0, Prefetch)]
    );
    assert!(reduce(&mut state, loaded(moved.next().unwrap())).is_empty());
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
        Arc::make_mut(range).commits.pop();
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
fn publication_accepts_prepared_ranges_when_a_newer_build_has_finished() {
    let (mut state, index) = fixture();
    request(&mut state, &index, vec![0]);
    state.repos[0].history_state.indexed.index = Some(
        HistoryIndexBuilder::new(HistorySnapshot("newer".into()), LogScope::AllBranches, 20)
            .unwrap()
            .finish(&CancellationToken::new())
            .unwrap(),
    );
    reduce(
        &mut state,
        Event::Publish {
            repo_id: RepoId(1),
            index: index.clone(),
        },
    );
    assert!(Arc::ptr_eq(
        state.repos[0]
            .history_state
            .indexed
            .displayed_index
            .as_ref()
            .unwrap(),
        &index
    ));

    state.repos[0].history_state.indexed.reset_query();
    reduce(
        &mut state,
        Event::Publish {
            repo_id: RepoId(1),
            index,
        },
    );
    assert!(
        state.repos[0]
            .history_state
            .indexed
            .displayed_index
            .is_none(),
        "a publication from a discarded query must be ignored"
    );
}

#[test]
fn indexed_regression_selection_uses_displayed_history_during_refresh() {
    let (mut state, index) = fixture();
    reduce(
        &mut state,
        Event::Publish {
            repo_id: RepoId(1),
            index: index.clone(),
        },
    );
    request(&mut state, &index, vec![9984]);
    // The next snapshot no longer contains the old visible commits.
    let replacement = HistoryIndexBuilder::new(
        HistorySnapshot("replacement".into()),
        LogScope::AllBranches,
        20,
    )
    .unwrap()
    .finish(&CancellationToken::new())
    .unwrap();
    state.repos[0].history_state.indexed.index = Some(replacement.clone());
    let previous_rev = state.repos[0].history_state.indexed.rev;
    request(&mut state, &replacement, vec![]);
    assert_ne!(
        state.repos[0].history_state.indexed.rev, previous_rev,
        "changing range sources must invalidate decoded card metadata"
    );
    let projection = HistoryProjection::new(index.clone(), vec![]);
    for (row, mode) in [
        (10_000, CommitSelectMode::Single),
        (10_002, CommitSelectMode::Range),
    ] {
        reduce(
            &mut state,
            Event::Select {
                request_id: None,
                repo_id: RepoId(1),
                projection: projection.clone(),
                commit_id: index.commit_id(row).unwrap(),
                mode,
            },
        );
        assert_eq!(
            state.repos[0].history_state.selected_commit,
            index.commit_id(row)
        );
    }
    assert_eq!(
        state.repos[0].history_state.multi_selection.commits.len(),
        3
    );
    let range = state.repos[0]
        .history_state
        .range_selection
        .as_ref()
        .expect("visible selection still opens a comparison");
    assert_eq!(range.to, index.commit_id(10_000));
    assert_eq!(range.from, index.commit_id(10_003).unwrap());

    reduce(
        &mut state,
        Event::Publish {
            repo_id: RepoId(1),
            index: replacement,
        },
    );
    let selected = state.repos[0].history_state.selected_commit.clone();
    reduce(
        &mut state,
        Event::Select {
            request_id: None,
            repo_id: RepoId(1),
            projection,
            commit_id: index.commit_id(10_004).unwrap(),
            mode: CommitSelectMode::Single,
        },
    );
    assert_eq!(
        state.repos[0].history_state.selected_commit, selected,
        "old presentation must be rejected after publication"
    );
}

#[test]
fn shift_selection_and_comparison_resolve_outside_the_bootstrap_page() {
    let (mut state, index) = fixture();
    reduce(
        &mut state,
        Event::Publish {
            repo_id: RepoId(1),
            index: index.clone(),
        },
    );
    request(&mut state, &index, vec![9984]);
    let projection = HistoryProjection::new(index.clone(), vec![10_002]);
    for (row, mode) in [
        (10_000, CommitSelectMode::Single),
        (10_004, CommitSelectMode::Range),
    ] {
        reduce(
            &mut state,
            Event::Select {
                request_id: None,
                repo_id: RepoId(1),
                projection: projection.clone(),
                commit_id: index.commit_id(row).unwrap(),
                mode,
            },
        );
    }
    assert_eq!(
        *state.repos[0].history_state.multi_selection.commits,
        [10_000, 10_001, 10_003, 10_004].map(|row| index.commit_id(row).unwrap())
    );
    assert!(state.repos[0].history_state.multi_selection.is_multi());
}
#[test]
fn a_selection_with_a_gap_opens_no_comparison_until_the_gap_closes() {
    let (mut state, index) = fixture();
    reduce(
        &mut state,
        Event::Publish {
            repo_id: RepoId(1),
            index: index.clone(),
        },
    );
    request(&mut state, &index, vec![9984]);
    let projection = HistoryProjection::new(index.clone(), vec![]);
    let select = |state: &mut AppState, row: usize, mode| {
        reduce(
            state,
            Event::Select {
                request_id: None,
                repo_id: RepoId(1),
                projection: projection.clone(),
                commit_id: index.commit_id(row).unwrap(),
                mode,
            },
        );
    };

    select(&mut state, 10_000, CommitSelectMode::Single);
    select(&mut state, 10_002, CommitSelectMode::Toggle);
    let pair = state.repos[0]
        .history_state
        .range_selection
        .as_ref()
        .expect("two commits compare directly");
    assert_eq!(pair.source, crate::model::ComparisonSource::SelectedPair);
    assert_eq!(pair.to, index.commit_id(10_000));
    assert_eq!(pair.from, index.commit_id(10_002).unwrap());

    select(&mut state, 10_003, CommitSelectMode::Toggle);
    assert_eq!(
        state.repos[0].history_state.multi_selection.commits.len(),
        3
    );
    assert_eq!(state.repos[0].history_state.range_selection, None);

    select(&mut state, 10_001, CommitSelectMode::Toggle);
    let range = state.repos[0]
        .history_state
        .range_selection
        .as_ref()
        .expect("consecutive commits open a comparison");
    assert_eq!(
        range.source,
        crate::model::ComparisonSource::MergedSelection
    );
    assert_eq!(range.to, index.commit_id(10_000));
    assert_eq!(range.from, index.commit_id(10_004).unwrap());
}

/// Rebuilds the index with only rows `0..2`, dropping the selected row 2,
/// over a comparison of `source`; returns what is left of the comparison
/// and the rebuild's effects.
fn rebuild_dropping_a_compared_commit(
    source: crate::model::ComparisonSource,
) -> (
    Option<crate::model::RangeSelection>,
    Vec<Effect>,
    Option<gitcomet_core::domain::DiffTarget>,
) {
    let (mut state, index) = fixture();
    let [newest, oldest] = [0, 2].map(|row| index.commit_id(row).unwrap());
    let repo = &mut state.repos[0];
    repo.history_state.multi_selection = crate::model::CommitMultiSelection {
        commits: vec![newest.clone(), oldest.clone()].into(),
        ..Default::default()
    };
    repo.history_state.selected_commit = Some(newest.clone());
    repo.history_state.range_selection = Some(crate::model::RangeSelection {
        source,
        ..crate::model::RangeSelection::new(
            oldest.clone(),
            Some(newest.clone()),
            "a".into(),
            "b".into(),
        )
    });
    repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::commit_range(
        oldest,
        Some(newest),
        Some("a.txt".into()),
    ));
    repo.history_state.indexed = Default::default();
    let snapshot = repo.history_state.log_snapshot.clone().unwrap();
    let work = reduce(&mut state, Event::Ensure { repo_id: RepoId(1) });
    let Some(Effect::IndexedHistory(Work::Build { seq, .. })) = work.first() else {
        panic!("expected a build");
    };
    let mut builder = HistoryIndexBuilder::new(snapshot, LogScope::AllBranches, 20).unwrap();
    for row in 0..2u32 {
        let mut id = [0u8; 20];
        id[..4].copy_from_slice(&row.to_be_bytes());
        let mut parent = [0u8; 20];
        parent[..4].copy_from_slice(&(row + 1).to_be_bytes());
        builder.push(&id, [parent.as_slice()], false).unwrap();
    }
    let built = builder.finish(&CancellationToken::new()).unwrap();
    let effects = reduce(
        &mut state,
        Event::Built {
            repo_id: RepoId(1),
            seq: *seq,
            result: Ok(Some(built)),
        },
    );
    (
        state.repos[0].history_state.range_selection.clone(),
        effects,
        state.repos[0].diff_state.diff_target.clone(),
    )
}

/// Rebuilds the fixture's index with only rows `0..rows` while `state`
/// holds its selection; returns the rebuild's effects.
fn rebuild_with_rows(state: &mut AppState, rows: u32) -> Vec<Effect> {
    let repo = &mut state.repos[0];
    repo.history_state.indexed = Default::default();
    let snapshot = repo.history_state.log_snapshot.clone().unwrap();
    let work = reduce(state, Event::Ensure { repo_id: RepoId(1) });
    let Some(Effect::IndexedHistory(Work::Build { seq, .. })) = work.first() else {
        panic!("expected a build");
    };
    let mut builder = HistoryIndexBuilder::new(snapshot, LogScope::AllBranches, 20).unwrap();
    for row in 0..rows {
        let mut id = [0u8; 20];
        id[..4].copy_from_slice(&row.to_be_bytes());
        let mut parent = [0u8; 20];
        parent[..4].copy_from_slice(&(row + 1).to_be_bytes());
        builder.push(&id, [parent.as_slice()], false).unwrap();
    }
    let built = builder.finish(&CancellationToken::new()).unwrap();
    reduce(
        state,
        Event::Built {
            repo_id: RepoId(1),
            seq: *seq,
            result: Ok(Some(built)),
        },
    )
}

#[test]
fn a_rebuild_that_leaves_two_commits_of_a_gap_compares_them() {
    let (mut state, index) = fixture();
    let [a, b, c] = [0, 2, 5].map(|row| index.commit_id(row).unwrap());
    let repo = &mut state.repos[0];
    repo.history_state.multi_selection = crate::model::CommitMultiSelection {
        commits: vec![a.clone(), b.clone(), c].into(),
        ..Default::default()
    };
    repo.history_state.selected_commit = Some(a.clone());

    rebuild_with_rows(&mut state, 4);

    let range = state.repos[0]
        .history_state
        .range_selection
        .clone()
        .expect("the surviving pair compares");
    assert_eq!(range.from, b);
    assert_eq!(range.to, Some(a));
    assert_eq!(range.source, crate::model::ComparisonSource::SelectedPair);
}

/// An explicit comparison empties the multi-selection but keeps the commit
/// clicked before it; that commit vanishing is no reason to end it.
#[test]
fn a_rebuild_that_drops_the_stray_selected_commit_keeps_an_explicit_comparison() {
    let (mut state, index) = fixture();
    let [tip, base, stray] = [0, 1, 5].map(|row| index.commit_id(row).unwrap());
    let repo = &mut state.repos[0];
    repo.history_state.multi_selection = crate::model::CommitMultiSelection::default();
    repo.history_state.selected_commit = Some(stray);
    repo.history_state.range_selection = Some(crate::model::RangeSelection::new(
        base,
        Some(tip),
        "base".into(),
        "tip".into(),
    ));

    rebuild_with_rows(&mut state, 4);

    assert!(state.repos[0].history_state.range_selection.is_some());
}

#[test]
fn a_rebuild_that_drops_a_compared_commit_leaves_the_selection_comparison() {
    use crate::model::ComparisonSource;
    for source in [
        ComparisonSource::SelectedPair,
        ComparisonSource::MergedSelection,
    ] {
        let (range, effects, diff_target) = rebuild_dropping_a_compared_commit(source);
        assert_eq!(range, None, "{source:?}");
        assert_eq!(
            diff_target, None,
            "the comparison's open diff goes too ({source:?})"
        );
        assert!(
                effects.iter().any(|e| matches!(
                    e,
                    Effect::LoadCommitDetails { commit_id, .. } if commit_id.as_ref() == "0000000000000000000000000000000000000000"
                )),
                "the details pane goes back to the still-selected commit ({source:?})"
            );
    }
    let (range, _, diff_target) = rebuild_dropping_a_compared_commit(ComparisonSource::Explicit);
    assert!(
        range.is_some(),
        "an explicit comparison does not come from the selection"
    );
    assert!(
        diff_target.is_some(),
        "an explicit comparison keeps its open diff"
    );
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

#[test]
fn progress_moves_the_status_revision_and_leaves_rows_alone() {
    let (mut state, _) = fixture();
    state.repos[0].history_state.indexed.index = None;
    let work = reduce(&mut state, Event::Ensure { repo_id: RepoId(1) }).remove(0);
    let Effect::IndexedHistory(Work::Build { seq, .. }) = work else {
        panic!()
    };
    let indexed = &state.repos[0].history_state.indexed;
    let (rev, status_rev) = (indexed.rev, indexed.status_rev);
    let progress = HistoryIndexProgress {
        scanned: 2048,
        matched: 2000,
    };
    reduce(
        &mut state,
        Event::Progress {
            repo_id: RepoId(1),
            seq,
            progress: progress.clone(),
        },
    );
    let indexed = &state.repos[0].history_state.indexed;
    assert_eq!(indexed.progress, Some(progress));
    assert_eq!(indexed.status_rev, status_rev.wrapping_add(1));
    assert_eq!(indexed.rev, rev);
}
