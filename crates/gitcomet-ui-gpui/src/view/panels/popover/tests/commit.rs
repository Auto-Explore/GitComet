use super::*;
use gitcomet_core::domain::RepoSpec;
use gitcomet_state::model::{AppState, Loadable, RepoState};

fn open_repo(repo_id: RepoId, workdir: &str) -> RepoState {
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: workdir.into(),
        },
    );
    repo.open = Loadable::Ready(());
    repo
}

#[gpui::test]
fn clean_repo_disables_commit_prompt_submission(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let repo_id = RepoId(1);
    let mut repo = open_repo(repo_id, "/tmp/clean-commit-prompt");
    repo.staged_status = Loadable::Ready(Arc::new(Vec::new()));
    store.replace_snapshot_for_test(Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..Default::default()
    }));

    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CommitPrompt { repo_id },
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
                host.commit_prompt_message_input
                    .update(cx, |input, cx| input.set_text("Commit message", cx));

                assert!(
                    !host.can_submit_commit_prompt(cx),
                    "expected a clean repo to disable the Commit button"
                );
                host.submit_commit_prompt(window, cx);
                assert!(
                    matches!(host.popover, Some(PopoverKind::CommitPrompt { .. })),
                    "expected disabled submission to keep the dialog open"
                );
            });
        });
    });
}

#[gpui::test]
fn commit_prompt_restores_drafts_per_repo(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let repo_a = RepoId(11);
    let repo_b = RepoId(12);
    store.replace_snapshot_for_test(Arc::new(AppState {
        repos: vec![
            open_repo(repo_a, "/tmp/commit-prompt-a"),
            open_repo(repo_b, "/tmp/commit-prompt-b"),
        ],
        active_repo: Some(repo_a),
        ..Default::default()
    }));

    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                let anchor = gpui::point(gpui::px(120.0), gpui::px(72.0));
                host.open_popover_at(
                    PopoverKind::CommitPrompt { repo_id: repo_a },
                    anchor,
                    window,
                    cx,
                );
                host.commit_prompt_message_input
                    .update(cx, |input, cx| input.set_text("repo A draft", cx));
                host.dismiss_prompt_popover(window, cx);

                host.open_popover_at(
                    PopoverKind::CommitPrompt { repo_id: repo_b },
                    anchor,
                    window,
                    cx,
                );
                assert_eq!(host.commit_prompt_message_input.read(cx).text(), "");
                host.commit_prompt_message_input
                    .update(cx, |input, cx| input.set_text("repo B draft", cx));
                host.dismiss_prompt_popover(window, cx);

                host.open_popover_at(
                    PopoverKind::CommitPrompt { repo_id: repo_a },
                    anchor,
                    window,
                    cx,
                );
                assert_eq!(
                    host.commit_prompt_message_input.read(cx).text(),
                    "repo A draft"
                );
                host.dismiss_prompt_popover(window, cx);

                host.open_popover_at(
                    PopoverKind::CommitPrompt { repo_id: repo_b },
                    anchor,
                    window,
                    cx,
                );
                assert_eq!(
                    host.commit_prompt_message_input.read(cx).text(),
                    "repo B draft"
                );
            });
        });
    });
}

#[gpui::test]
fn successful_commit_prompt_submission_clears_draft(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let repo_id = RepoId(21);
    let mut repo = open_repo(repo_id, "/tmp/commit-prompt-submit");
    repo.staged_status = Loadable::Ready(Arc::new(Vec::new()));
    repo.merge_commit_message = Loadable::Ready(Some("Merge branch 'topic'".to_string()));
    store.replace_snapshot_for_test(Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..Default::default()
    }));

    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                let anchor = gpui::point(gpui::px(120.0), gpui::px(72.0));
                host.open_popover_at(PopoverKind::CommitPrompt { repo_id }, anchor, window, cx);
                host.commit_prompt_message_input
                    .update(cx, |input, cx| input.set_text("finish merge", cx));
                assert!(host.can_submit_commit_prompt(cx));
                host.submit_commit_prompt(window, cx);

                host.open_popover_at(PopoverKind::CommitPrompt { repo_id }, anchor, window, cx);
                assert_eq!(host.commit_prompt_message_input.read(cx).text(), "");
            });
        });
    });
}
