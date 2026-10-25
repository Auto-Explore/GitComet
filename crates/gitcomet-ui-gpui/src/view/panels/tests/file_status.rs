use super::*;

fn leaked(selector: String) -> &'static str {
    Box::leak(selector.into_boxed_str())
}

fn repo_with_unstaged_paths(
    repo_id: gitcomet_state::model::RepoId,
    paths: &[&str],
) -> gitcomet_state::model::RepoState {
    let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-status-tree"));
    repo.worktree_status = gitcomet_state::model::Loadable::Ready(Arc::new(
        paths
            .iter()
            .map(|path| gitcomet_core::domain::FileStatus {
                path: (*path).into(),
                kind: gitcomet_core::domain::FileStatusKind::Modified,
                conflict: None,
            })
            .collect(),
    ));
    repo.worktree_status_rev = 1;
    repo
}

/// Renders the details pane for a commit with an author, optionally carrying a
/// signature verdict, and returns the drawn view.
fn commit_details_signature_fixture(
    cx: &mut gpui::TestAppContext,
    signature: Option<gitcomet_core::domain::CommitSignature>,
) -> (
    gpui::Entity<crate::view::GitCometView>,
    &mut gpui::VisualTestContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(34);
    let commit_sha = "0123456789abcdef0123456789abcdef01234567".to_string();
    let commit_id = gitcomet_core::domain::CommitId(commit_sha.clone().into());

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-commit-signature"));
            repo.history_state.selected_commit = Some(commit_id.clone());
            repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::CommitDetails {
                    id: commit_id.clone(),
                    message: "subject".to_string(),
                    author_name: "Ada Lovelace".to_string(),
                    author_email: "ada@example.com".to_string(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".to_string(),
                    committed_at_unix: 0,
                    parent_ids: vec![],
                    files: vec![],
                },
            ));
            if let Some(signature) = signature {
                let mut map = rustc_hash::FxHashMap::default();
                map.insert(commit_id.clone(), signature);
                repo.history_state.commit_signatures = Arc::new(map.into_iter().collect());
                // The details pane is fingerprint-gated: without this the
                // snapshot is skipped and the badge never draws.
                repo.history_state.commit_signatures_rev = 1;
            }

            let next_state = app_state_with_repo(repo, repo_id);
            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    (view, cx)
}

fn test_signature(
    status: gitcomet_core::domain::SignatureStatus,
) -> gitcomet_core::domain::CommitSignature {
    gitcomet_core::domain::CommitSignature {
        status,
        format: gitcomet_core::domain::SignatureFormat::OpenPgp,
        signer: Some(Arc::from("Ada Lovelace <ada@example.com>")),
        key_id: Some(Arc::from("DEADBEEFDEADBEEF")),
    }
}

mod commit_details;
mod commit_form;
mod file_lists;
mod list_layouts;
mod previews;
mod section_layout;
mod status_actions;
