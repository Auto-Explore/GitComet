//! Routing and the command palette: commands, reveal commit, push/pull targets, activation.

use super::*;

#[test]
fn status_section_shortcuts_leave_modified_app_and_text_chords_alone() {
    for chord in ["ctrl-a", "secondary-a", "ctrl-s", "secondary-u", "space"] {
        assert!(
            is_status_section_shortcut(&gpui::Keystroke::parse(chord).unwrap()),
            "{chord}"
        );
    }
    for chord in [
        "a",
        "s",
        "ctrl-shift-a",
        "secondary-shift-a",
        "alt-a",
        "alt-space",
        "f4",
        "secondary-f",
    ] {
        assert!(
            !is_status_section_shortcut(&gpui::Keystroke::parse(chord).unwrap()),
            "{chord}"
        );
    }
}

#[test]
fn recent_repository_shortcut_is_not_a_diff_select_all_candidate() {
    let recent = gpui::Keystroke::parse("secondary-shift-a").expect("valid shortcut");
    let select_all = gpui::Keystroke::parse("secondary-a").expect("valid shortcut");

    assert!(
        !is_diff_shortcut_candidate(&recent),
        "the app-level recent-repositories chord must not reach diff text selection"
    );
    assert!(
        is_diff_shortcut_candidate(&select_all),
        "unshifted Ctrl/Cmd+A must still reach diff text selection"
    );
}

fn open_reveal_commit_dialog(cx: &mut gpui::VisualTestContext, view: &gpui::Entity<GitCometView>) {
    cx.simulate_keystrokes("secondary-g");
    test_support::redraw(cx);
    assert!(
        reveal_commit_is_open(cx, view),
        "expected secondary-g to open the Go to dialog"
    );
}

fn commit_lookup(store: &AppStore) -> gitcomet_state::model::CommitLookup {
    store.snapshot().repos[0]
        .history_state
        .commit_lookup
        .clone()
}

fn wait_for_commit_lookup(store: &AppStore, repo_id: RepoId, reference: &str) {
    // GPUI's executor does not drive the store's worker thread. These fixtures
    // have no open backend repository, so wait for the request, not a Git reply.
    wait_until("store lookup for the current commit reference", || {
        let lookup = repo_commit_lookup(store, repo_id);
        lookup.reference.as_ref().map(|id| id.as_ref()) == Some(reference)
    });
}

fn repo_commit_lookup(store: &AppStore, repo_id: RepoId) -> gitcomet_state::model::CommitLookup {
    store
        .snapshot()
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .unwrap_or_else(|| panic!("repo {repo_id:?} in snapshot"))
        .history_state
        .commit_lookup
        .clone()
}

fn command_palette_input_focus(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
) -> Option<gpui::FocusHandle> {
    cx.update(|_window, app| {
        Some(
            view.read(app)
                .command_palette
                .read(app)
                .query_input
                .read(app)
                .focus_handle(),
        )
    })
}

#[test]
fn push_request_uses_configured_upstream_without_claiming_it_is_live() {
    let repo = repo_with_push_state(
        Some(Upstream {
            remote: "origin".to_string(),
            branch: "feature".to_string(),
        }),
        Loadable::Loading,
    );

    assert_eq!(push_request(&repo, &Default::default()), PushRequest::Push);
    assert!(!head_branch_has_live_upstream(&repo));
}

#[test]
fn live_upstream_requires_the_exact_loaded_remote_tracking_ref() {
    let mut repo = repo_with_push_state(
        Some(Upstream {
            remote: "origin".to_string(),
            branch: "feature".to_string(),
        }),
        Loadable::Ready(Arc::new(vec![Remote {
            name: "origin".to_string(),
            url: None,
        }])),
    );
    repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
        remote: "origin".to_string(),
        name: "feature".to_string(),
        target: CommitId("remote-feature".into()),
    }]));

    assert!(head_branch_has_live_upstream(&repo));
    assert!(pull_enabled(&repo, &Default::default()));
    assert_eq!(pull_request(&repo, &Default::default()), PullRequest::Pull);
}

#[test]
fn configured_but_unpushed_upstream_is_a_push_target_without_being_live() {
    let mut repo = repo_with_push_state(
        Some(Upstream {
            remote: "origin".to_string(),
            branch: "review/feature".to_string(),
        }),
        Loadable::Ready(Arc::new(vec![Remote {
            name: "origin".to_string(),
            url: None,
        }])),
    );
    repo.remote_branches = Loadable::Ready(Arc::new(Vec::new()));

    assert_eq!(push_request(&repo, &Default::default()), PushRequest::Push);
    assert!(!head_branch_has_live_upstream(&repo));
    assert!(!pull_enabled(&repo, &Default::default()));
    assert_eq!(
        pull_request(&repo, &Default::default()),
        PullRequest::NotReady,
        "Pull must stay disabled until the configured branch exists remotely"
    );
}

#[test]
fn push_request_offers_standard_remote_for_untracked_branch() {
    let repo = repo_with_push_state(
        None,
        Loadable::Ready(Arc::new(vec![
            Remote {
                name: "backup".to_string(),
                url: None,
            },
            Remote {
                name: "origin".to_string(),
                url: None,
            },
        ])),
    );

    assert_eq!(
        push_request(&repo, &Default::default()),
        PushRequest::SetUpstream {
            remote: "origin".to_string()
        }
    );
    assert!(!head_branch_has_live_upstream(&repo));
}

#[test]
fn push_request_uses_first_remote_when_origin_is_absent() {
    let repo = repo_with_push_state(
        None,
        Loadable::Ready(Arc::new(vec![Remote {
            name: "upstream".to_string(),
            url: None,
        }])),
    );

    assert_eq!(
        push_request(&repo, &Default::default()),
        PushRequest::SetUpstream {
            remote: "upstream".to_string()
        }
    );
}

/// An annex adjusted branch never has an upstream. Offering to set one would
/// publish the adjusted branch; Push goes through `git annex push` instead.
#[test]
fn push_request_on_an_annex_adjusted_branch_never_offers_set_upstream() {
    let mut repo = repo_with_push_state(
        None,
        Loadable::Ready(Arc::new(vec![Remote {
            name: "origin".to_string(),
            url: None,
        }])),
    );
    let head = "adjusted/main(unlocked)".to_string();
    repo.head_branch = Loadable::Ready(head.clone());
    repo.branches = Loadable::Ready(Arc::new(vec![Branch {
        name: head,
        target: CommitId("deadbeef".into()),
        upstream: None,
        divergence: None,
    }]));
    let mut support = gitcomet_core::large_files::LargeFileSupport::default();
    support.annex.uuid = Some("u".into());
    repo.large_file_support = Loadable::Ready(Arc::new(support));

    assert_eq!(push_request(&repo, &Default::default()), PushRequest::Push);
    let settings = gitcomet_state::model::LargeFileSettings {
        annex_pull_push: false,
        ..Default::default()
    };
    assert_eq!(
        push_request(&repo, &settings),
        PushRequest::SetUpstream {
            remote: "origin".into()
        }
    );
    for support in [
        Loadable::NotLoaded,
        Loadable::Loading,
        Loadable::Error("failed".into()),
    ] {
        repo.large_file_support = support;
        assert_eq!(push_request(&repo, &Default::default()), PushRequest::Push);
    }
    repo.large_file_support = Loadable::Ready(Arc::default());
    assert_eq!(
        push_request(&repo, &Default::default()),
        PushRequest::SetUpstream {
            remote: "origin".into()
        }
    );
}

#[test]
fn push_request_distinguishes_no_remotes_from_loading_data() {
    let no_remotes = repo_with_push_state(None, Loadable::Ready(Arc::new(Vec::new())));
    let loading = repo_with_push_state(None, Loadable::Loading);

    assert_eq!(
        push_request(&no_remotes, &Default::default()),
        PushRequest::NoRemotes
    );
    assert_eq!(
        push_request(&loading, &Default::default()),
        PushRequest::NotReady
    );
}

#[test]
fn a_selected_remote_branch_is_invalidated_only_after_a_ready_refresh_omits_it() {
    let repo_id = RepoId(1);
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/selected-remote-branch"),
        },
    );
    let selected = SelectedBranch {
        repo_id,
        target: BranchMenuTarget::remote("origin", "deleted"),
    };
    let mut state = AppState {
        repos: vec![repo.clone()],
        active_repo: Some(repo_id),
        ..AppState::test_default()
    };

    assert!(
        !selected_remote_branch_is_missing(&state, Some(&selected)),
        "loading data is not proof that the selection disappeared"
    );

    repo.remote_branches = Loadable::Ready(Arc::new(Vec::new()));
    state.repos[0] = repo;
    assert!(selected_remote_branch_is_missing(&state, Some(&selected)));
}

#[test]
fn pull_request_offers_a_pull_for_a_branch_that_was_never_pushed() {
    let repo = repo_with_push_state(
        None,
        Loadable::Ready(Arc::new(vec![Remote {
            name: "origin".to_string(),
            url: None,
        }])),
    );

    assert!(!head_branch_has_live_upstream(&repo));
    assert!(!pull_enabled(&repo, &Default::default()));
    assert_eq!(
        pull_request(&repo, &Default::default()),
        PullRequest::Pull,
        "the backend pulls from the preferred remote and sets the upstream"
    );
}

#[test]
fn pull_request_allows_a_detached_head_and_reports_a_repo_without_remotes() {
    let mut detached = repo_with_push_state(None, Loadable::Loading);
    detached.head_branch = Loadable::Ready("HEAD".to_string());
    assert!(head_is_detached(&detached));
    assert_eq!(
        pull_request(&detached, &Default::default()),
        PullRequest::Pull
    );

    let no_remotes = repo_with_push_state(None, Loadable::Ready(Arc::new(Vec::new())));
    assert_eq!(
        pull_request(&no_remotes, &Default::default()),
        PullRequest::NoRemotes
    );
}

#[test]
fn a_selected_remote_branch_survives_a_remote_name_containing_a_slash() {
    let repo_id = RepoId(1);
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/nested-remote"),
        },
    );
    repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
        remote: "forks/alice".to_string(),
        name: "main".to_string(),
        target: CommitId("deadbeef".into()),
    }]));
    let state = AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..AppState::test_default()
    };
    let selected = SelectedBranch {
        repo_id,
        target: BranchMenuTarget::remote("forks/alice", "main"),
    };

    assert!(!selected_remote_branch_is_missing(&state, Some(&selected)));
}

#[gpui::test]
fn command_palette_opens_from_detached_focus_on_loading_repo_tabs(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    install_app_shortcuts_for_test(cx, Arc::clone(&backend));
    install_repo_tab_test_state(&store, &view, cx, RepoId(1));
    focus_detached_window_focus(cx);

    cx.simulate_keystrokes("secondary-p");
    test_support::redraw(cx);

    assert!(
        command_palette_is_open(cx, &view),
        "expected secondary-p from detached focus to open the command palette"
    );
    assert!(
        cx.debug_bounds("modal_scrim").is_some(),
        "expected command palette to use the shared modal scrim"
    );
    let input_focus = command_palette_input_focus(cx, &view)
        .expect("expected command palette input to exist after opening");
    cx.update(|window, app| {
        assert_eq!(
            window.focused(app),
            Some(input_focus),
            "expected command palette input to own window focus after opening"
        );
    });
}

#[gpui::test]
fn command_palette_reopens_after_tab_switch_and_close_cycles(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    install_app_shortcuts_for_test(cx, Arc::clone(&backend));
    install_repo_tab_test_state(&store, &view, cx, RepoId(1));
    store.dispatch(Msg::SetActiveRepo { repo_id: RepoId(2) });
    sync_view_snapshot(cx, &view);

    cx.simulate_keystrokes("secondary-p");
    test_support::redraw(cx);
    assert!(
        command_palette_is_open(cx, &view),
        "expected command palette to open after switching repository tabs"
    );

    cx.simulate_keystrokes("secondary-p");
    test_support::redraw(cx);
    assert!(
        !command_palette_is_open(cx, &view),
        "expected secondary-p to close the command palette"
    );

    cx.simulate_keystrokes("secondary-p");
    test_support::redraw(cx);
    assert!(
        command_palette_is_open(cx, &view),
        "expected command palette to reopen after a toggle-close cycle"
    );

    cx.simulate_keystrokes("escape");
    test_support::redraw(cx);
    assert!(
        !command_palette_is_open(cx, &view),
        "expected escape to close the command palette"
    );

    cx.simulate_keystrokes("secondary-p");
    test_support::redraw(cx);
    assert!(
        command_palette_is_open(cx, &view),
        "expected command palette to reopen after closing with escape"
    );
}

#[gpui::test]
fn command_palette_opens_commit_prompt_for_clean_repo(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    install_app_shortcuts_for_test(cx, Arc::clone(&backend));
    cx.update(|_window, app| crate::app::bind_text_input_keys_for_test(app));
    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.repos[0].staged_status = Loadable::Ready(Arc::new(Vec::new()));
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    cx.simulate_keystrokes("secondary-p");
    test_support::redraw(cx);
    cx.simulate_keystrokes("enter");
    test_support::redraw(cx);

    cx.update(|_window, app| {
        assert!(
            matches!(
                test_support::popover_kind(view.read(app), app),
                Some(PopoverKind::CommitPrompt { repo_id: RepoId(1) })
            ),
            "expected Commit Changes to remain selectable for a clean repo"
        );
    });
    assert!(
        cx.debug_bounds("modal_scrim").is_some(),
        "expected command-palette dialogs to use the shared modal scrim"
    );
}

#[gpui::test]
fn command_palette_rename_branch_opens_prompt_for_current_branch(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.repos[0].head_branch = Loadable::Ready("feature/current".to_string());
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.execute_command("rename-branch", Some(window), cx)
        });
    });
    test_support::redraw(cx);

    cx.update(|_window, app| {
        assert!(matches!(
            test_support::popover_kind(view.read(app), app),
            Some(PopoverKind::RenameBranchPrompt {
                repo_id: RepoId(1),
                name,
                is_current_branch: true,
            }) if name == "feature/current"
        ));
    });
}

/// Staging is what marks a conflict resolved, so every stage entry point has to
/// warn about markers left in the worktree — including the command palette's
/// "Stage all", which reaches conflicted files just as the buttons do.
#[gpui::test]
fn command_palette_stage_all_asks_before_staging_unresolved_conflicts(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_command_stage_all_conflict",
        std::process::id()
    ));
    let conflicted = PathBuf::from("conflicted.rs");
    std::fs::create_dir_all(&workdir).unwrap();
    std::fs::write(
        workdir.join(&conflicted),
        "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> other\nb\n",
    )
    .unwrap();

    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.repos[0].spec.workdir = workdir.clone();
    state.repos[0].status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: std::sync::Arc::new(vec![]),
            unstaged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                path: conflicted.clone(),
                kind: gitcomet_core::domain::FileStatusKind::Modified,
                conflict: Some(gitcomet_core::domain::FileConflictKind::BothModified),
            }]),
        }
        .into(),
    );
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    let ops_rev_before = test_support::repo_ops_rev(&view, cx, RepoId(1));
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.execute_command("stage-all", Some(window), cx)
        });
    });
    test_support::redraw(cx);

    cx.update(|_window, app| {
        let kind = test_support::popover_kind(view.read(app), app);
        assert!(
            matches!(
                kind,
                Some(PopoverKind::StageConflictMarkersConfirm { ref unresolved, .. })
                    if unresolved == &vec![conflicted.clone()]
            ),
            "expected the unresolved-conflict confirmation, got {kind:?}"
        );
    });

    // The stage itself must wait for the user's answer.
    test_support::drain_store_worker(&view, cx);
    assert_eq!(
        test_support::repo_ops_rev(&view, cx, RepoId(1)),
        ops_rev_before,
        "nothing may be staged until the confirmation is answered"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
fn command_palette_close_falls_back_to_diff_panel_when_saved_focus_is_stale(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    install_app_shortcuts_for_test(cx, Arc::clone(&backend));
    let state = view_state_with_active_ready_repo(RepoId(1));
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    cx.update(|window, app| {
        let focus = view
            .read(app)
            .main_pane
            .read(app)
            .diff_panel_focus_handle
            .clone();
        window.focus(&focus, app);
        let _ = window.draw(app);
    });
    test_support::redraw(cx);

    cx.simulate_keystrokes("secondary-p");
    test_support::redraw(cx);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let stale_focus = this
                .command_palette
                .read(cx)
                .query_input
                .read(cx)
                .focus_handle();
            this.command_palette.update(cx, |palette, _cx| {
                palette.restore_focus = Some(stale_focus);
            });
        });
    });

    cx.simulate_keystrokes("secondary-p");
    test_support::redraw(cx);
    pump_for(cx, Duration::from_millis(16));

    let diff_focus = cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .diff_panel_focus_handle
            .clone()
    });
    cx.update(|window, app| {
        assert_eq!(
            window.focused(app),
            Some(diff_focus),
            "expected stale command-palette restore focus to fall back to the diff panel"
        );
    });
}

#[test]
fn window_activation_dispatches_repo_activated_message() {
    let repo_id = RepoId(1);
    let state = view_state_with_active_ready_repo(repo_id);
    let mut last_activation_dispatch = FxHashMap::default();
    let now = Instant::now();

    let msg = repo_activation_msg(&state, &mut last_activation_dispatch, now)
        .expect("ready active repo should produce activation message");

    assert!(matches!(msg, Msg::RepoActivated { repo_id: got } if got == repo_id));
    assert!(!matches!(msg, Msg::RepoExternallyChanged { .. }));
}

#[test]
fn window_activation_dispatch_is_throttled_per_repo() {
    let repo_id = RepoId(1);
    let state = view_state_with_active_ready_repo(repo_id);
    let mut last_activation_dispatch = FxHashMap::default();
    let now = Instant::now();

    assert!(repo_activation_msg(&state, &mut last_activation_dispatch, now).is_some());
    assert!(
        repo_activation_msg(
            &state,
            &mut last_activation_dispatch,
            now + Duration::from_secs(1),
        )
        .is_none()
    );
    assert!(matches!(
        repo_activation_msg(
            &state,
            &mut last_activation_dispatch,
            now + REPO_ACTIVATION_THROTTLE,
        ),
        Some(Msg::RepoActivated { repo_id: got }) if got == repo_id
    ));
}

#[test]
fn opening_repository_satisfies_initial_window_activation_refresh() {
    let repo_id = RepoId(1);
    let state = view_state_with_active_ready_repo(repo_id);
    let mut recent = FxHashMap::default();
    let now = Instant::now();
    note_opened_repos_for_activation(&AppState::test_default(), &state, &mut recent, now);
    assert!(repo_activation_msg(&state, &mut recent, now).is_none());

    // Unrelated snapshots must not postpone a genuine later focus refresh.
    note_opened_repos_for_activation(&state, &state, &mut recent, now + REPO_ACTIVATION_THROTTLE);
    assert!(repo_activation_msg(&state, &mut recent, now + REPO_ACTIVATION_THROTTLE).is_some());
    note_opened_repos_for_activation(&state, &AppState::test_default(), &mut recent, now);
    assert!(
        recent.is_empty(),
        "closed repository IDs must not accumulate"
    );
}

#[test]
fn opening_repository_does_not_throttle_focus_until_its_load_starts() {
    let repo_id = RepoId(1);
    let mut state = view_state_with_active_ready_repo(repo_id);
    state.repos[0].open = Loadable::Loading;
    let mut recent = FxHashMap::default();
    let now = Instant::now();
    note_opened_repos_for_activation(&AppState::test_default(), &state, &mut recent, now);
    assert!(recent.is_empty());
    let ready = view_state_with_active_ready_repo(repo_id);
    note_opened_repos_for_activation(&state, &ready, &mut recent, now);
    assert!(repo_activation_msg(&ready, &mut recent, now).is_none());
}

#[test]
fn window_grab_suppresses_the_activation_it_caused() {
    // Dragging the title bar or a resize edge hands focus to the compositor for
    // the duration of the grab, which GPUI reports as a deactivate → activate
    // pair. Treating that as a return to the app refreshed the whole repo on
    // every window move/resize.
    let now = Instant::now();
    crate::app::note_window_grab_started();

    let armed = crate::app::take_window_grab_started_within(now, WINDOW_GRAB_DEACTIVATE_GRACE);
    assert!(armed, "a fresh grab must claim the deactivation it caused");

    let mut suppressed_at = Some(now);
    assert!(consume_window_grab_activation(
        &mut suppressed_at,
        now + Duration::from_secs(5)
    ));
    assert!(
        suppressed_at.is_none(),
        "the marker must be consumed so it cannot suppress twice"
    );
}

#[test]
fn stale_window_grab_does_not_suppress_a_later_activation() {
    // A grab the compositor ignored (bad serial, unsupported protocol) must not
    // leave suppression armed for an unrelated alt-tab minutes later.
    let now = Instant::now();
    crate::app::note_window_grab_started();

    assert!(!crate::app::take_window_grab_started_within(
        now + WINDOW_GRAB_DEACTIVATE_GRACE + Duration::from_millis(1),
        WINDOW_GRAB_DEACTIVATE_GRACE,
    ));
    assert!(
        !crate::app::take_window_grab_started_within(now, WINDOW_GRAB_DEACTIVATE_GRACE),
        "the stale marker must have been cleared, not left armed"
    );
}

#[test]
fn window_grab_suppression_expires_for_a_very_late_activation() {
    let now = Instant::now();
    let mut suppressed_at = Some(now);
    assert!(!consume_window_grab_activation(
        &mut suppressed_at,
        now + WINDOW_GRAB_REACTIVATE_GRACE + Duration::from_secs(1),
    ));
}

#[test]
fn unsuppressed_activation_still_dispatches_repo_activated() {
    // Suppression is opt-in, and a suppressed activation must not stamp the
    // throttle map — a genuine alt-tab right after a drag still refreshes.
    let repo_id = RepoId(1);
    let state = view_state_with_active_ready_repo(repo_id);
    let mut last_activation_dispatch = FxHashMap::default();
    let now = Instant::now();

    let mut suppressed_at = None;
    assert!(!consume_window_grab_activation(&mut suppressed_at, now));
    assert!(matches!(
        repo_activation_msg(&state, &mut last_activation_dispatch, now),
        Some(Msg::RepoActivated { repo_id: got }) if got == repo_id
    ));
}

#[test]
fn branch_sidebar_sorts_unsorted_remote_branches() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::new(),
        },
    );

    repo.remote_branches = Loadable::Ready(Arc::new(vec![
        RemoteBranch {
            remote: "upstream".to_string(),
            name: "zeta/topic".to_string(),
            target: CommitId("deadbeef".into()),
        },
        RemoteBranch {
            remote: "origin".to_string(),
            name: "feature/topic".to_string(),
            target: CommitId("feedface".into()),
        },
        RemoteBranch {
            remote: "origin".to_string(),
            name: "alpha".to_string(),
            target: CommitId("cafebabe".into()),
        },
        RemoteBranch {
            remote: "origin".to_string(),
            name: "feature".to_string(),
            target: CommitId("8badf00d".into()),
        },
        RemoteBranch {
            remote: "origin".to_string(),
            name: "alpha".to_string(),
            target: CommitId("decafbad".into()),
        },
        RemoteBranch {
            remote: "upstream".to_string(),
            name: "main".to_string(),
            target: CommitId("facefeed".into()),
        },
    ]));

    let rows = GitCometView::branch_sidebar_rows(&repo);
    let names = rows
        .iter()
        .filter_map(|row| match row {
            BranchSidebarRow::Branch {
                section: BranchSection::Remote,
                name,
                ..
            } => Some(name.as_ref().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        vec![
            "origin/feature",
            "origin/feature/topic",
            "origin/alpha",
            "upstream/zeta/topic",
            "upstream/main",
        ]
    );
}

#[test]
fn branch_sidebar_starts_with_local_and_remote_branch_sections() {
    let repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::new(),
        },
    );

    let rows = GitCometView::branch_sidebar_rows(&repo);
    assert!(
        matches!(
            rows.first(),
            Some(BranchSidebarRow::SectionHeader {
                section: BranchSection::Local,
                ..
            })
        ),
        "expected Local Branches header to be the first sidebar row"
    );
    assert!(
        rows.iter().any(|row| matches!(
            row,
            BranchSidebarRow::SectionHeader {
                section: BranchSection::Remote,
                ..
            }
        )),
        "expected Remote branches header to be present"
    );
}

#[test]
fn focused_mergetool_keeps_titlebar_actions_without_repo_tabs_or_command_palette() {
    assert!(titlebar_repo_tab_actions_enabled(
        GitCometViewMode::FocusedMergetool,
        true
    ));
    assert!(!show_titlebar_repo_tabs(GitCometViewMode::FocusedMergetool));
    assert!(!command_palette_available(
        GitCometViewMode::FocusedMergetool
    ));

    assert!(show_titlebar_repo_tabs(GitCometViewMode::Normal));
    assert!(command_palette_available(GitCometViewMode::Normal));
}

#[gpui::test]
fn window_deactivation_clears_live_pointer_feedback(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, _| window.activate());
    test_support::redraw(cx);
    cx.simulate_mouse_down(
        point(px(300.0), px(300.0)),
        gpui::MouseButton::Left,
        Default::default(),
    );
    cx.update(|window, app| assert!(crate::press_gesture::pointer_is_down(window, app)));
    cx.deactivate_window();
    cx.update(|window, app| assert!(!crate::press_gesture::pointer_is_down(window, app)));
}

/// Switching repository tabs while the dialog is open leaves the typed query
/// pointing at the *new* repository, whose lookup slot has never been asked
/// about it. Nothing else will ask until the user edits the query, so without a
/// re-request the row sits on "Resolving…" forever.
#[gpui::test]
fn reveal_commit_reissues_its_lookup_against_a_newly_active_repository(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    install_app_shortcuts_for_test(cx, Arc::clone(&backend));
    install_repo_tab_test_state(&store, &view, cx, RepoId(1));
    open_reveal_commit_dialog(cx, &view);

    cx.simulate_input("deadbee");
    cx.run_until_parked();
    wait_for_commit_lookup(&store, RepoId(1), "deadbee");
    test_support::redraw(cx);
    assert_eq!(
        repo_commit_lookup(&store, RepoId(1)).reference,
        Some(CommitId("deadbee".into())),
        "the active repository should have been asked about the typed reference"
    );

    // Switch tabs by publishing the snapshot directly: `dispatch` hands the
    // message to the store's own worker thread, which `run_until_parked` (a
    // gpui-executor barrier) does not wait for.
    let mut switched = (*store.snapshot()).clone();
    switched.active_repo = Some(RepoId(2));
    store.replace_snapshot_for_test(Arc::new(switched));
    sync_view_snapshot(cx, &view);
    cx.run_until_parked();
    wait_for_commit_lookup(&store, RepoId(2), "deadbee");
    test_support::redraw(cx);

    assert_eq!(
        repo_commit_lookup(&store, RepoId(2)).reference,
        Some(CommitId("deadbee".into())),
        "the query must be re-asked of the repository that is now active"
    );
}

/// The palette and the dialog paint on the same overlay layer, each with its
/// own scrim. Opening one over the other would stack two scrims and strand the
/// lower modal when the upper is dismissed.
#[gpui::test]
fn reveal_commit_and_the_command_palette_never_stack(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    install_app_shortcuts_for_test(cx, Arc::clone(&backend));
    install_repo_tab_test_state(&store, &view, cx, RepoId(1));

    cx.simulate_keystrokes("secondary-p");
    test_support::redraw(cx);
    assert!(command_palette_is_open(cx, &view), "palette should open");

    cx.simulate_keystrokes("secondary-g");
    test_support::redraw(cx);
    assert!(
        reveal_commit_is_open(cx, &view),
        "the dialog should open over the palette"
    );
    assert!(
        !command_palette_is_open(cx, &view),
        "opening the dialog must close the palette rather than stack on it"
    );

    // And the other direction.
    cx.simulate_keystrokes("secondary-p");
    test_support::redraw(cx);
    assert!(command_palette_is_open(cx, &view), "palette should reopen");
    assert!(
        !reveal_commit_is_open(cx, &view),
        "opening the palette must close the dialog"
    );

    cx.simulate_keystrokes("escape");
    test_support::redraw(cx);
    assert!(
        !command_palette_is_open(cx, &view) && !reveal_commit_is_open(cx, &view),
        "escape should leave nothing open"
    );
}

#[gpui::test]
fn reveal_commit_dialog_opens_on_secondary_g_and_takes_focus(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    install_app_shortcuts_for_test(cx, Arc::clone(&backend));
    install_repo_tab_test_state(&store, &view, cx, RepoId(1));

    open_reveal_commit_dialog(cx, &view);
    assert!(
        cx.debug_bounds("modal_scrim").is_some(),
        "expected the dialog to use the shared modal scrim"
    );
    assert!(
        cx.debug_bounds("reveal_commit_title").is_some(),
        "expected the Go to title"
    );
    assert!(
        cx.debug_bounds("reveal_commit_examples").is_some(),
        "an empty query should show the examples"
    );

    let input_focus = cx.update(|_window, app| {
        view.read(app)
            .reveal_commit_dialog
            .read(app)
            .query_input
            .read(app)
            .focus_handle()
    });
    cx.update(|window, app| {
        assert_eq!(
            window.focused(app),
            Some(input_focus),
            "expected the query input to own window focus after opening"
        );
    });
}

/// Both close paths have to leave the dialog reopenable. Escape goes through the
/// input's transient-key flag while the chord goes through the action, so they
/// can drift apart.
#[gpui::test]
fn reveal_commit_dialog_toggles_and_escapes_without_latching(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    install_app_shortcuts_for_test(cx, Arc::clone(&backend));
    install_repo_tab_test_state(&store, &view, cx, RepoId(1));

    open_reveal_commit_dialog(cx, &view);

    cx.simulate_keystrokes("secondary-g");
    test_support::redraw(cx);
    assert!(
        !reveal_commit_is_open(cx, &view),
        "expected secondary-g to close the dialog"
    );

    open_reveal_commit_dialog(cx, &view);

    cx.simulate_keystrokes("escape");
    test_support::redraw(cx);
    assert!(
        !reveal_commit_is_open(cx, &view),
        "expected escape to close the dialog"
    );

    open_reveal_commit_dialog(cx, &view);
}

/// A lookup is a git call, so a single character must not spawn one; two
/// already can be a tag, and that is where asking starts.
#[gpui::test]
fn reveal_commit_asks_git_only_once_the_query_could_be_a_reference(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    install_app_shortcuts_for_test(cx, Arc::clone(&backend));
    install_repo_tab_test_state(&store, &view, cx, RepoId(1));
    open_reveal_commit_dialog(cx, &view);

    cx.simulate_input("d");
    cx.run_until_parked();
    test_support::redraw(cx);
    assert_eq!(
        commit_lookup(&store).reference,
        None,
        "a single character must not send git looking for a reference"
    );
    assert!(
        cx.debug_bounds("reveal_commit_examples").is_some(),
        "the examples stay up until there is something to look up"
    );

    cx.simulate_input("eadbee");
    cx.run_until_parked();
    wait_for_commit_lookup(&store, RepoId(1), "deadbee");
    test_support::redraw(cx);
    assert_eq!(
        commit_lookup(&store).reference,
        Some(CommitId("deadbee".into())),
        "the current query should be the one being resolved"
    );
    assert!(
        cx.debug_bounds("reveal_commit_examples").is_none(),
        "the examples give way once a lookup is under way"
    );
}

/// The point of the preview is that Enter reveals the *resolved* commit: the
/// full id, so the history walk matches loaded rows outright.
#[gpui::test]
fn reveal_commit_enter_reveals_the_resolved_full_id(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    install_app_shortcuts_for_test(cx, Arc::clone(&backend));
    install_repo_tab_test_state(&store, &view, cx, RepoId(1));
    open_reveal_commit_dialog(cx, &view);

    // `install_app_shortcuts_for_test` binds only the app chords; Enter belongs
    // to the TextInput context, which the real app binds separately.
    cx.update(|window, app| {
        app.bind_keys([gpui::KeyBinding::new(
            "enter",
            crate::kit::Enter,
            Some("TextInput"),
        )]);
        let _ = window.draw(app);
    });

    cx.simulate_input("deadbee");
    cx.run_until_parked();
    wait_for_commit_lookup(&store, RepoId(1), "deadbee");
    test_support::redraw(cx);

    // Stand in for the backend answering the lookup the typing just issued.
    let full = CommitId("deadbeef0123456789abcdef0123456789abcdef".into());
    let mut state = (*store.snapshot()).clone();
    let lookup = &mut state.repos[0].history_state.commit_lookup;
    lookup.result = gitcomet_state::model::Loadable::Ready(gitcomet_core::domain::Commit {
        id: full.clone(),
        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
        summary: "the reland".into(),
        author: "Test User".into(),
        time: std::time::SystemTime::UNIX_EPOCH,
    });
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    assert!(
        cx.debug_bounds("reveal_commit_match").is_some(),
        "expected the resolved commit to be offered as a row"
    );

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    test_support::redraw(cx);

    assert!(
        !reveal_commit_is_open(cx, &view),
        "activating a result should close the dialog"
    );
    assert_eq!(
        store.snapshot().repos[0]
            .history_state
            .reveal_target
            .as_ref(),
        Some(&full),
        "the reveal should target the full id, not the abbreviation that was typed"
    );
}

fn open_palette_on_ready_repo(
    cx: &mut gpui::TestAppContext,
    merging: bool,
) -> (
    AppStore,
    gpui::Entity<GitCometView>,
    &mut gpui::VisualTestContext,
) {
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    install_app_shortcuts_for_test(cx, Arc::clone(&backend));
    cx.update(|_window, app| crate::app::bind_text_input_keys_for_test(app));
    let mut state = view_state_with_active_ready_repo(RepoId(1));
    if merging {
        state.repos[0].merge_commit_message =
            Loadable::Ready(Some("Merge branch 'feature'".to_string()));
    }
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    cx.simulate_keystrokes("secondary-p");
    test_support::redraw(cx);
    (store, view, cx)
}

/// Asked for explicitly: a command that cannot run right now stays listed,
/// greyed out, with a hover tooltip saying why — and Enter does nothing.
#[gpui::test]
fn command_palette_keeps_unavailable_commands_listed_with_a_reason(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (_store, view, cx) = open_palette_on_ready_repo(cx, false);

    cx.simulate_input("abort merge");
    test_support::redraw(cx);

    let row = cx
        .debug_bounds("command_palette_disabled_abort-merge")
        .expect("Abort Merge should be listed, disabled, with no merge in progress");
    assert!(
        cx.debug_bounds("command_palette_unavailable_reason")
            .is_some(),
        "the keyboard-selected disabled row should say why in place"
    );

    cx.simulate_mouse_move(row.center(), None, gpui::Modifiers::default());
    test_support::wait_for_native_tooltip(cx);
    assert_eq!(
        test_support::tooltip_text(cx, &view).map(|text| text.to_string()),
        Some("Only available while a merge is in progress".to_string()),
        "hovering the disabled row should explain why it is disabled"
    );

    cx.simulate_keystrokes("enter");
    test_support::redraw(cx);
    assert!(
        command_palette_is_open(cx, &view),
        "Enter on a disabled command must not run it or close the palette"
    );
    cx.update(|_window, app| {
        assert!(
            test_support::popover_kind(view.read(app), app).is_none(),
            "no abort confirmation should open"
        );
    });
}

/// The same command becomes live exactly when its state holds, and runs
/// through the action bar's own confirmation.
#[gpui::test]
fn command_palette_enables_abort_merge_during_a_merge(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (_store, view, cx) = open_palette_on_ready_repo(cx, true);

    cx.simulate_input("abort merge");
    test_support::redraw(cx);
    assert!(
        cx.debug_bounds("command_palette_disabled_abort-merge")
            .is_none(),
        "Abort Merge should be enabled while a merge is in progress"
    );

    cx.simulate_keystrokes("enter");
    test_support::redraw(cx);
    cx.update(|_window, app| {
        assert!(
            matches!(
                test_support::popover_kind(view.read(app), app),
                Some(PopoverKind::MergeAbortConfirm { repo_id: RepoId(1) })
            ),
            "Abort Merge should open the same confirmation as the action bar"
        );
    });
}
