use super::*;
use gitcomet_core::domain::{LogPage, LogScope, RepoSpec};
use gitcomet_core::history_index::{HistoryIndexBuilder, HistoryRange};
use gitcomet_core::services::{CancellationToken, HistorySnapshot};
use gitcomet_state::model::{CommitMultiSelection, RangeSelection};

fn fixture(selected: &[usize]) -> (RepoState, Vec<Commit>) {
    let mut builder = HistoryIndexBuilder::new(
        HistorySnapshot("comparison".into()),
        LogScope::AllBranches,
        20,
    )
    .unwrap();
    let commits: Vec<_> = (0..600u32)
        .map(|row| {
            let mut id = [0u8; 20];
            id[..4].copy_from_slice(&row.to_be_bytes());
            builder
                .push(&id, std::iter::empty(), row as i64, false)
                .unwrap();
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
            .collect(),
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
        builder.push(&id, std::iter::empty(), 0, false).unwrap();
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
