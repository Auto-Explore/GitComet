use super::*;
use gitcomet_core::domain::{Branch, RemoteBranch, RepoSpec};
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let repo_id = RepoId(1);
    let mut repo = open_repo(repo_id, "/tmp/clean-commit-prompt");
    repo.staged_status = Loadable::Ready(Arc::new(Vec::new()));
    store.replace_snapshot_for_test(Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..AppState::test_default()
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let repo_a = RepoId(11);
    let repo_b = RepoId(12);
    store.replace_snapshot_for_test(Arc::new(AppState {
        repos: vec![
            open_repo(repo_a, "/tmp/commit-prompt-a"),
            open_repo(repo_b, "/tmp/commit-prompt-b"),
        ],
        active_repo: Some(repo_a),
        ..AppState::test_default()
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let repo_id = RepoId(21);
    let mut repo = open_repo(repo_id, "/tmp/commit-prompt-submit");
    repo.staged_status = Loadable::Ready(Arc::new(Vec::new()));
    repo.merge_commit_message = Loadable::Ready(Some("Merge branch 'topic'".to_string()));
    store.replace_snapshot_for_test(Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..AppState::test_default()
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

/// The "Merge into current" entry and the dialog it opens must agree.
///
/// The entry used to resolve the destination through `active_repo()`, which is
/// `None` for a commit belonging to any *other* open repository, so with two
/// repositories open the menu read "into HEAD" while the confirmation it opened
/// -- which resolves through `state.repos` -- read "into release". The same
/// split left the entry enabled for a repository the dialog was about to
/// refuse.
#[gpui::test]
fn merge_entry_names_and_gates_on_the_commits_own_repository(cx: &mut gpui::TestAppContext) {
    fn merge_entry(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<GitCometView>,
        repo_id: RepoId,
        commit_id: &CommitId,
    ) -> (String, bool) {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.context_menu_model(
                        &PopoverKind::CommitMenu {
                            repo_id,
                            commit_id: commit_id.clone(),
                        },
                        cx,
                    )
                })
            })
        })
        .expect("expected a commit context menu model")
        .items
        .iter()
        .find_map(|item| match item {
            ContextMenuItem::Entry {
                label, disabled, ..
            } if label.starts_with("Merge ") => Some((label.to_string(), *disabled)),
            _ => None,
        })
        .expect("expected the merge entry")
    }

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let active_id = RepoId(1);
    let other_id = RepoId(2);
    let commit_id = CommitId("0123456789abcdef".into());

    let apply = |cx: &mut gpui::VisualTestContext, other_busy: bool| {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                let mut active = open_repo(active_id, "/tmp/merge-entry-active");
                active.head_branch = Loadable::Ready("active-branch".into());
                let mut other = open_repo(other_id, "/tmp/merge-entry-other");
                other.head_branch = Loadable::Ready("release".into());
                other.local_actions_in_flight = u32::from(other_busy);

                let state = Arc::new(AppState {
                    repos: vec![active, other],
                    active_repo: Some(active_id),
                    ..AppState::test_default()
                });
                this.state = Arc::clone(&state);
                this.ui_model
                    .update(cx, |model, cx| model.set_state(state, cx));
                cx.notify();
            });
        });
    };

    apply(cx, false);

    let (active_label, _) = merge_entry(cx, &view, active_id, &commit_id);
    assert!(
        active_label.ends_with(" into active-branch"),
        "the active repository still names its own head: {active_label:?}"
    );

    let (other_label, other_disabled) = merge_entry(cx, &view, other_id, &commit_id);
    assert!(
        other_label.ends_with(" into release"),
        "the entry must name the destination its confirmation dialog will: {other_label:?}"
    );
    assert!(
        !other_disabled,
        "an idle repository's commit is mergeable from any menu"
    );

    // And readiness comes from that same repository, not from the active one.
    apply(cx, true);
    let (_, other_disabled) = merge_entry(cx, &view, other_id, &commit_id);
    assert!(
        other_disabled,
        "a busy repository's entry must be disabled, not offered and then refused"
    );
    let (_, active_disabled) = merge_entry(cx, &view, active_id, &commit_id);
    assert!(
        !active_disabled,
        "the other repository being busy says nothing about this one"
    );
}

#[gpui::test]
fn copy_branch_name_entries_follow_the_commits_branches(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    // The active tab is a different, empty repository: the entries must come
    // from the repository the menu was opened for.
    let active_id = RepoId(2);
    let commit_id = CommitId("0123456789abcdef".into());
    let other_commit = CommitId("fedcba9876543210".into());
    let local = |name: &str, target: &CommitId| Branch {
        name: name.into(),
        target: target.clone(),
        upstream: None,
        divergence: None,
    };
    let remote = |name: &str, target: &CommitId| RemoteBranch {
        remote: "origin".into(),
        name: name.into(),
        target: target.clone(),
    };

    let copy_entries = |cx: &mut gpui::VisualTestContext,
                        branches: Vec<Branch>,
                        remote_branches: Vec<RemoteBranch>| {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                let mut repo = open_repo(repo_id, "/tmp/copy-branch-name");
                repo.branches = Loadable::Ready(Arc::new(branches));
                repo.remote_branches = Loadable::Ready(Arc::new(remote_branches));
                let state = Arc::new(AppState {
                    repos: vec![repo, open_repo(active_id, "/tmp/copy-branch-name-active")],
                    active_repo: Some(active_id),
                    ..AppState::test_default()
                });
                this.state = Arc::clone(&state);
                this.ui_model
                    .update(cx, |model, cx| model.set_state(state, cx));
                cx.notify();
            });
        });
        // The popover host picks up the new state once that update ends.
        let model = cx
            .update(|_window, app| {
                view.update(app, |this, cx| {
                    this.popover_host.update(cx, |host, cx| {
                        host.context_menu_model(
                            &PopoverKind::CommitMenu {
                                repo_id,
                                commit_id: commit_id.clone(),
                            },
                            cx,
                        )
                    })
                })
            })
            .expect("expected a commit context menu model");
        let labels: Vec<String> = model
            .items
            .iter()
            .filter_map(|item| match item {
                ContextMenuItem::Entry { label, .. } => Some(label.to_string()),
                _ => None,
            })
            .collect();
        // The copy entries sit directly under "Copy commit SHA".
        if let Some(first) = labels
            .iter()
            .position(|l| l.starts_with("Copy branch name"))
        {
            assert_eq!(labels[first - 1], "Copy commit SHA");
        }
        model
            .items
            .iter()
            // No ref group is expanded here, so these are only the commit-level
            // entries, not the "Copy branch name" inside a branch's own group.
            .filter_map(|item| match item {
                ContextMenuItem::Entry { label, action, .. }
                    if label.starts_with("Copy branch name") =>
                {
                    match action.as_ref() {
                        ContextMenuAction::CopyText { text } => {
                            Some((label.to_string(), text.clone()))
                        }
                        _ => panic!("{label} must copy text"),
                    }
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };

    // One local branch: a plain entry, preferred over its remote-tracking ref.
    assert_eq!(
        copy_entries(
            cx,
            vec![
                local("feat/app-cert-check", &commit_id),
                local("main", &other_commit)
            ],
            vec![remote("feat/app-cert-check", &commit_id)],
        ),
        vec![("Copy branch name".into(), "feat/app-cert-check".into())]
    );

    // Several local branches: one entry each, named.
    assert_eq!(
        copy_entries(
            cx,
            vec![local("feat/a", &commit_id), local("feat/b", &commit_id)],
            Vec::new(),
        ),
        vec![
            ("Copy branch name feat/a".into(), "feat/a".into()),
            ("Copy branch name feat/b".into(), "feat/b".into()),
        ]
    );

    // Only a remote branch: copy it the way its own menu does.
    assert_eq!(
        copy_entries(cx, Vec::new(), vec![remote("feat/x", &commit_id)]),
        vec![("Copy branch name".into(), "origin/feat/x".into())]
    );

    // No branch at this commit: no entry.
    assert_eq!(
        copy_entries(cx, vec![local("main", &other_commit)], Vec::new()),
        Vec::<(String, String)>::new()
    );
}
