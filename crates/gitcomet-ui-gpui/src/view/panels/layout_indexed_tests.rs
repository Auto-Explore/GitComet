use super::*;
use gitcomet_core::domain::{LogPage, LogScope, RepoSpec};
use gitcomet_core::history_index::{HistoryIndexBuilder, HistoryRange};
use gitcomet_core::services::{CancellationToken, HistorySnapshot};
use gitcomet_state::model::{CommitMultiSelection, RangeSelection};

fn fixture(selected: &[usize]) -> (RepoState, Vec<Commit>) {
    fixture_with_count(selected, 600)
}

fn fixture_with_count(selected: &[usize], count: u32) -> (RepoState, Vec<Commit>) {
    let mut builder = HistoryIndexBuilder::new(
        HistorySnapshot("comparison".into()),
        LogScope::AllBranches,
        20,
    )
    .unwrap();
    let commits: Vec<_> = (0..count)
        .map(|row| {
            let mut id = [0u8; 20];
            id[..4].copy_from_slice(&row.to_be_bytes());
            builder.push(&id, std::iter::empty(), false).unwrap();
            Commit {
                id: CommitId(gitcomet_core::hex::encode(&id).into()),
                parent_ids: Default::default(),
                summary: format!("commit {row}").into(),
                author: "Alice".into(),
                time: std::time::UNIX_EPOCH,
            }
        })
        .collect();
    let index = builder.finish(&CancellationToken::new()).unwrap();
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: "/tmp/indexed-comparison".into(),
        },
    );
    repo.log = Loadable::Ready(Arc::new(LogPage {
        commits: commits[..200].to_vec(),
        next_cursor: None,
    }));
    repo.history_state.indexed.index = Some(index.clone());
    repo.history_state.indexed.range_index = Some(index.clone());
    repo.history_state.indexed.ranges.insert(
        256,
        Arc::new(HistoryRange {
            snapshot: index.snapshot.clone(),
            start: 256,
            commits: commits[256..512].to_vec(),
        }),
    );
    repo.history_state.multi_selection = CommitMultiSelection {
        commits: selected
            .iter()
            .map(|&row| commits[row].id.clone())
            .collect::<Vec<_>>()
            .into(),
        ..Default::default()
    };
    repo.history_state.range_selection = Some(RangeSelection {
        from: commits[301].id.clone(),
        to: Some(commits[300].id.clone()),
        from_label: "base".into(),
        to_label: "tip".into(),
    });
    (repo, commits)
}

#[test]
fn indexed_regression_comparison_cards_resolve_beyond_and_across_bootstrap_page() {
    for selected in [&[301, 300][..], &[301, 100, 300][..]] {
        let (repo, commits) = fixture(selected);
        let mut ordered = selected.to_vec();
        ordered.sort_unstable();
        let expected: Vec<_> = ordered.iter().map(|&row| commits[row].clone()).collect();
        for ids in [
            DetailsPaneView::multi_selected_commit_ids_in_log_order(&repo),
            DetailsPaneView::range_comparison_commit_ids(&repo),
        ] {
            let resolved: Vec<_> = DetailsPaneView::comparison_commits(&repo, &ids)
                .into_iter()
                .flatten()
                .cloned()
                .collect();
            assert_eq!(resolved, expected);
        }
    }
}

#[test]
fn indexed_regression_comparison_endpoint_cards_resolve_from_ranges() {
    let (repo, commits) = fixture(&[]);
    let ids = DetailsPaneView::range_comparison_commit_ids(&repo);
    let resolved: Vec<_> = DetailsPaneView::comparison_commits(&repo, &ids)
        .into_iter()
        .flatten()
        .cloned()
        .collect();
    assert_eq!(resolved, vec![commits[300].clone(), commits[301].clone()]);
}

#[test]
fn indexed_regression_comparison_handoff_keeps_order_when_range_rows_move() {
    let (mut repo, commits) = fixture(&[301, 300]);
    repo.history_state.indexed.displayed_index = repo.history_state.indexed.index.clone();
    let mut builder = HistoryIndexBuilder::new(
        HistorySnapshot("replacement".into()),
        LogScope::AllBranches,
        20,
    )
    .unwrap();
    for row in [301u32, 300] {
        let mut id = [0u8; 20];
        id[..4].copy_from_slice(&row.to_be_bytes());
        builder.push(&id, std::iter::empty(), false).unwrap();
    }
    let replacement = builder.finish(&CancellationToken::new()).unwrap();
    let indexed = &mut repo.history_state.indexed;
    indexed.index = Some(replacement.clone());
    indexed.range_index = Some(replacement.clone());
    indexed.ranges.clear();
    indexed.ranges.insert(
        0,
        Arc::new(HistoryRange {
            snapshot: replacement.snapshot.clone(),
            start: 0,
            commits: vec![commits[301].clone(), commits[300].clone()],
        }),
    );
    let ids = DetailsPaneView::range_comparison_commit_ids(&repo);
    assert_eq!(ids, [commits[300].id.clone(), commits[301].id.clone()]);
    let resolved: Vec<_> = DetailsPaneView::comparison_commits(&repo, &ids)
        .into_iter()
        .flatten()
        .cloned()
        .collect();
    assert_eq!(resolved, [commits[300].clone(), commits[301].clone()]);
}

#[gpui::test]
fn indexed_regression_comparison_count_and_cache_survive_metadata_loading(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (repo, _) = fixture(&[301, 100, 300]);
    let (store, events) =
        gitcomet_state::store::AppStore::new(Arc::new(crate::view::test_support::TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|_, app| {
        let pane = view.read(app).details_pane.read(app);
        let mut loading = repo.clone();
        loading.history_state.indexed.ranges.clear();
        let pending = pane.range_comparison_commits_shared(&loading);
        assert_eq!(
            pending.len(),
            3,
            "selected commits still need cards when their metadata is not cached"
        );
        assert!(std::rc::Rc::ptr_eq(
            &pending,
            &pane.range_comparison_commits_shared(&loading)
        ));
        let mut loaded = repo;
        loaded.history_state.indexed.rev += 1;
        let ready = pane.range_comparison_commits_shared(&loaded);
        assert!(
            !std::rc::Rc::ptr_eq(&pending, &ready),
            "hydration must invalidate comparison cards without a log or selection change"
        );
        assert_eq!(
            ready
                .iter()
                .map(|card| card.summary.as_ref())
                .collect::<Vec<_>>(),
            ["commit 100", "commit 300", "commit 301"]
        );
    });
}

#[gpui::test]
fn indexed_history_comparison_ignores_progress_and_unrelated_blocks(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::history_perf::{self, Work};
    let _guard = crate::test_support::lock_visual_test();
    let (mut repo, commits) = fixture(&[300, 301]);
    let (store, events) =
        gitcomet_state::store::AppStore::new(Arc::new(crate::view::test_support::TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|_, app| {
        let pane = view.read(app).details_pane.read(app);
        let cards = pane.range_comparison_commits_shared(&repo);
        let _capture = history_perf::capture();
        repo.history_state.indexed.rev += 1;
        repo.history_state.indexed.ranges.insert(
            0,
            Arc::new(HistoryRange {
                snapshot: repo
                    .history_state
                    .indexed
                    .index
                    .as_ref()
                    .unwrap()
                    .snapshot
                    .clone(),
                start: 0,
                commits: commits[..256].to_vec(),
            }),
        );
        assert!(std::rc::Rc::ptr_eq(
            &cards,
            &pane.range_comparison_commits_shared(&repo)
        ));
        assert_eq!(history_perf::count(Work::ComparisonCard), 0);
    });
}

#[gpui::test]
fn indexed_history_large_comparisons_prepare_order_once_and_only_build_visible_cards(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::history_perf::{self, Work};
    let _guard = crate::test_support::lock_visual_test();
    let (mut repo, commits) = fixture_with_count(&[], 100_000);
    repo.history_state.multi_selection.commits = Arc::new(
        commits
            .iter()
            .rev()
            .map(|commit| commit.id.clone())
            .collect(),
    );
    let (store, events) =
        gitcomet_state::store::AppStore::new(Arc::new(crate::view::test_support::TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let pane = cx.update(|_, app| view.read(app).details_pane.clone());
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            pane.state = Arc::new(gitcomet_state::model::AppState {
                repos: vec![repo.clone()],
                active_repo: Some(repo.id),
                ..Default::default()
            });
            pane.ensure_comparison_order(cx);
            assert!(pane.comparison_order.is_none());
            assert!(pane.comparison_order_pending.is_some());
        })
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        pane.update(app, |pane, cx| {
            let ordered = pane
                .comparison_order
                .as_ref()
                .expect("background order published")
                .ids
                .clone();
            assert_eq!(ordered.len(), 100_000);
            assert_eq!(ordered[0], commits[0].id);
            assert_eq!(ordered[99_999], commits[99_999].id);
            let _capture = history_perf::capture();
            let visible = pane.comparison_cards(&repo, &ordered[1000..1040], 1000);
            assert_eq!(visible.len(), 40);
            assert_eq!(history_perf::count(Work::ComparisonCard), 40);
            repo.history_state.indexed.rev += 1;
            pane.state = Arc::new(gitcomet_state::model::AppState {
                repos: vec![repo.clone()],
                active_repo: Some(repo.id),
                ..Default::default()
            });
            pane.ensure_comparison_order(cx);
            assert!(Arc::ptr_eq(
                &ordered,
                &pane.comparison_order.as_ref().unwrap().ids
            ));
            assert!(pane.comparison_order_pending.is_none());
            assert!(std::rc::Rc::ptr_eq(
                &visible,
                &pane.comparison_cards(&repo, &ordered[1000..1040], 1000)
            ));
            assert_eq!(history_perf::count(Work::ComparisonCard), 40);
        })
    });
}
