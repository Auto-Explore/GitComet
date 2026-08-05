use super::*;

/// A range comparison started via "mark + compare" (or a branch/tag/worktree
/// compare) leaves `multi_selection` empty — the endpoints live only in
/// `range_selection`. The compared-commit preview cards must still render,
/// resolved from those endpoints by looking each SHA up in the log.
///
/// Regression guard: the cards were built with a virtualized `uniform_list`
/// inside a fixed-height, non-scrolling container, which painted nothing and
/// left only the "Viewing diff between N commits" subheader visible.
#[gpui::test]
fn range_comparison_renders_endpoint_commit_cards(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(77);
    let from_sha = "0000000000000000000000000000000000000000";
    let to_sha = "1111111111111111111111111111111111111111";

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-range-compare"));
            repo.open = gitcomet_state::model::Loadable::Ready(());
            repo.head_branch = gitcomet_state::model::Loadable::Ready("main".into());
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus::default().into(),
            );
            repo.log = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::LogPage {
                    // Newest first, matching real log order: tip then base.
                    commits: vec![
                        gitcomet_core::domain::Commit {
                            id: gitcomet_core::domain::CommitId(to_sha.into()),
                            parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                            summary: "tip commit".into(),
                            author: "Alice".into(),
                            time: std::time::SystemTime::UNIX_EPOCH,
                        },
                        gitcomet_core::domain::Commit {
                            id: gitcomet_core::domain::CommitId(from_sha.into()),
                            parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                            summary: "base commit".into(),
                            author: "Alice".into(),
                            time: std::time::SystemTime::UNIX_EPOCH,
                        },
                    ],
                    next_cursor: None,
                },
            ));
            repo.log_rev = 1;
            // The mark + compare flow: a range selection with no multi-selection.
            repo.history_state.range_selection = Some(gitcomet_state::model::RangeSelection {
                from: gitcomet_core::domain::CommitId(from_sha.into()),
                to: Some(gitcomet_core::domain::CommitId(to_sha.into())),
                from_label: "0000000".into(),
                to_label: "1111111".into(),
            });
            repo.history_state.range_files = gitcomet_state::model::Loadable::NotLoaded;

            let next_state = app_state_with_repo(repo, repo_id);
            this.store.replace_snapshot_for_test(Arc::clone(&next_state));
            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert!(
        cx.debug_bounds("commit_multi_row_0").is_some(),
        "expected the tip commit's preview card to render for a range comparison"
    );
    assert!(
        cx.debug_bounds("commit_multi_row_1").is_some(),
        "expected the base commit's preview card to render for a range comparison"
    );
}
