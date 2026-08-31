use super::*;
use crate::view::panels::tests::{app_state_with_repo, opening_repo_state};

#[gpui::test]
fn browse_history_menu_exposes_full_commit_message_tooltip(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let commit_id = CommitId("deadbeefdeadbeef".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_browse_history_tooltip",
        std::process::id()
    ));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.log = Loadable::Ready(
                gitcomet_core::domain::LogPage {
                    commits: vec![gitcomet_core::domain::Commit {
                        id: commit_id.clone(),
                        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                        summary: "Fix the thing".into(),
                        author: "Alice".into(),
                        time: SystemTime::UNIX_EPOCH,
                    }],
                    next_cursor: None,
                }
                .into(),
            );
            repo.navigation.browse_history = vec![commit_id.clone()];

            let state = app_state_with_repo(repo, repo_id);
            this.state = Arc::clone(&state);
            this.ui_model
                .update(cx, |model, cx| model.set_state(state, cx));
            cx.notify();
        });
    });

    cx.update(|_window, app| {
        let model = view
            .update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.context_menu_model(&PopoverKind::BrowseHistoryMenu { repo_id }, cx)
                })
            })
            .expect("expected browse-history context menu model");

        let entry_ix = model
            .items
            .iter()
            .position(|item| {
                matches!(
                    item,
                    ContextMenuItem::Entry { action, .. }
                        if matches!(
                            action.as_ref(),
                            ContextMenuAction::BrowseRepositoryAtCommit { commit_id: c, .. }
                                if *c == commit_id
                        )
                )
            })
            .expect("expected an entry for the browsed commit");

        assert_eq!(
            model.entry_tooltips.get(&entry_ix).map(|t| t.as_ref()),
            Some("Fix the thing"),
            "browse-history entry should expose the full commit message as a tooltip"
        );
    });
}
