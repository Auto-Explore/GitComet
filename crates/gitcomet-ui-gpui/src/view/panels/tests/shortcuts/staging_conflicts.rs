//! Staging and conflict shortcuts: space/Ctrl+S staging, conflict navigation.

use super::*;

#[gpui::test]
fn semantic_conflict_navigation_handles_automatic_deltas_and_projection_rebuilds(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70544);
    let commit_id = CommitId("1122334455667722".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_semantic_conflict_nav",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/semantic-conflict.rs");
    let base = "start\nold-a\nsep-1\nold-conflict-1\nsep-2\nold-b\nsep-3\nold-conflict-2\nend\n";
    let ours = "start\nnew-a\nsep-1\nours-conflict-1\nsep-2\nold-b\nsep-3\nours-conflict-2\nend\n";
    let theirs =
        "start\nold-a\nsep-1\ntheirs-conflict-1\nsep-2\nnew-b\nsep-3\ntheirs-conflict-2\nend\n";
    let session = ConflictSession::from_stage_inputs(
        path.clone(),
        gitcomet_core::domain::FileConflictKind::BothModified,
        ConflictPayload::Text(base.into()),
        ConflictPayload::Text(ours.into()),
        ConflictPayload::Text(theirs.into()),
    );
    let current = session
        .marker_projection_text()
        .expect("plan-backed session marker projection")
        .to_string();

    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    set_test_conflict_status(&mut repo, path.clone(), DiffArea::Unstaged);
    set_test_conflict_file(&mut repo, path.clone(), base, ours, theirs, current);
    repo.conflict_state.conflict_file_load_mode = gitcomet_state::model::ConflictFileLoadMode::Full;
    repo.conflict_state.conflict_session = Some(session);
    repo.conflict_state.conflict_rev = 1;

    apply_state(cx, &view, app_state_with_active_repo(repo));
    wait_for_main_pane_condition(
        cx,
        &view,
        "semantic conflict targets",
        |pane| {
            pane.conflict_resolver.path.as_deref() == Some(path.as_path())
                && pane.conflict_resolver.nav_targets.len() == 4
                && pane.conflict_resolver.active_conflict == Some(0)
                && pane
                    .conflict_resolver
                    .nav_anchor
                    .is_some_and(|anchor| anchor.order_hint == 1)
        },
        |pane| {
            format!(
                "path={:?} targets={:?} anchor={:?} active={:?}",
                pane.conflict_resolver.path,
                pane.conflict_resolver.nav_targets,
                pane.conflict_resolver.nav_anchor,
                pane.conflict_resolver.active_conflict,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                assert!(pane.conflict_has_prev_delta());
                assert!(pane.conflict_has_next_delta());
                pane.conflict_jump_first(cx);
            });
        });
    });
    let (anchor, active, can_prev_conflict, can_next_conflict, can_first) =
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            (
                pane.conflict_resolver.nav_anchor,
                pane.conflict_resolver.active_conflict,
                pane.conflict_has_prev(),
                pane.conflict_has_next(),
                pane.conflict_has_prev_delta(),
            )
        });
    assert_eq!(anchor.unwrap().order_hint, 0);
    assert_eq!(active, None, "automatic deltas have no marker block");
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let (has_base, selected) = pane
            .conflict_resolver_active_pick_state()
            .expect("the semantic automatic delta remains actionable");
        assert!(has_base);
        assert!(selected.contains(&crate::view::conflict_resolver::ConflictChoice::Ours));
    });

    // Ctrl+3 reaches the semantic plan block even though navigation left no
    // displayed marker selected. KDiff3-style source picks toggle, so the
    // automatic local selection becomes an ordered Local+Remote selection.
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_detached_window_focus(cx);
    cx.simulate_keystrokes("ctrl-3");
    wait_for_main_pane_condition(
        cx,
        &view,
        "automatic delta Ctrl+3 override",
        |pane| {
            pane.conflict_resolver_active_pick_state()
                .is_some_and(|(_, selected)| {
                    selected.contains(&crate::view::conflict_resolver::ConflictChoice::Ours)
                        && selected
                            .contains(&crate::view::conflict_resolver::ConflictChoice::Theirs)
                })
        },
        |pane| {
            format!(
                "active pick state={:?}",
                pane.conflict_resolver_active_pick_state()
            )
        },
    );
    assert!(!can_prev_conflict);
    assert!(can_next_conflict);
    assert!(!can_first);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_jump_next(cx);
            });
        });
    });
    assert_eq!(active_conflict_ix(cx, &view), 0);
    assert_eq!(conflict_navigation_anchor(cx, &view), Some(1));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_jump_last(cx);
            });
        });
    });
    assert_eq!(active_conflict_ix(cx, &view), 1);
    assert_eq!(conflict_navigation_anchor(cx, &view), Some(3));
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(pane.conflict_has_prev_delta());
        assert!(!pane.conflict_has_next_delta());
    });

    // A block click resets the semantic anchor before subsequent F2/F3/F7
    // traversal, even when another target was selected previously.
    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            pane.conflict_resolver_select_conflict(0, cx);
            pane.conflict_resolver_toggle_collapse_context(cx);
            pane.conflict_resolver_toggle_hide_resolved(cx);
            let next_mode = match pane.conflict_resolver.view_mode {
                ConflictResolverViewMode::ThreeWay => ConflictResolverViewMode::TwoWayDiff,
                ConflictResolverViewMode::TwoWayDiff => ConflictResolverViewMode::ThreeWay,
            };
            pane.conflict_resolver_set_view_mode(next_mode, cx);
        });
    });
    assert_eq!(active_conflict_ix(cx, &view), 0);
    assert_eq!(
        conflict_navigation_anchor(cx, &view),
        Some(1),
        "view mode, context folding, and hide-resolved rebuilds preserve the anchor"
    );

    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_detached_window_focus(cx);
    cx.simulate_keystrokes("f7");
    draw_and_drain_test_window(cx);
    assert_eq!(
        active_conflict_ix(cx, &view),
        1,
        "detached-focus navigation continues from the clicked semantic target"
    );
    assert_eq!(conflict_navigation_anchor(cx, &view), Some(3));

    // Resolving the last conflict keeps the existing wrap-around auto-advance
    // behavior, but the destination is selected through the semantic target
    // list (and therefore skips both automatic deltas).
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            pane.conflict_resolver_pick_active_conflict(
                crate::view::conflict_resolver::ConflictChoice::Ours,
                cx,
            );
        });
    });
    assert_eq!(
        active_conflict_ix(cx, &view),
        0,
        "auto-advance wraps from the last resolved conflict to the first unresolved conflict"
    );
    assert_eq!(conflict_navigation_anchor(cx, &view), Some(1));

    // Ctrl+Shift+3 is Choose C Everywhere, not "all unresolved conflicts".
    // It must replace both original conflicts and both automatic deltas.
    cx.update(|_window, app| {
        main_pane.update(app, |pane, cx| {
            pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::ThreeWay, cx);
        });
    });
    cx.simulate_keystrokes("ctrl-shift-3");
    // The bulk choice lands in the store, and this harness seeds the view's
    // state directly rather than wiring the store through to it, so assert
    // where the reducer actually writes.
    let delta_selections = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            let snapshot = view.read(app).store.snapshot();
            snapshot
                .repos
                .iter()
                .find_map(|repo| repo.conflict_state.conflict_session.as_ref())
                .and_then(|session| session.merge_plan.as_ref())
                .map(|plan| {
                    plan.blocks
                        .iter()
                        .filter(|block| block.is_delta)
                        .map(|block| block.selection.as_slice().to_vec())
                        .collect::<Vec<_>>()
                })
        })
    };
    wait_until(cx, "Choose C Everywhere", |cx| {
        delta_selections(cx).is_some_and(|blocks| {
            !blocks.is_empty()
                && blocks
                    .iter()
                    .all(|selection| selection.as_slice() == [gitcomet_core::merge::MergeSource::C])
        })
    });
}

#[gpui::test]
fn diff_editor_staging_context_menu_restores_diff_panel_focus_for_f4(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70560);
    let commit_id = CommitId("abcdef0011223344".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_editor_stage_focus",
        std::process::id()
    ));
    let first = std::path::PathBuf::from("src/first.rs");
    let second = std::path::PathBuf::from("src/second.rs");
    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        &[first.clone(), second.clone()],
        &first,
    );

    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_diff_panel(cx, &view);
    open_popover_for_test(
        cx,
        &view,
        PopoverKind::DiffEditorMenu {
            repo_id,
            area: DiffArea::Unstaged,
            path: Some(first.clone()),
            hunk_patch: Some("diff --git a/src/first.rs b/src/first.rs\n".into()),
            hunks_count: 1,
            lines_patch: Some("diff --git a/src/first.rs b/src/first.rs\n".into()),
            discard_lines_patch: None,
            lines_count: 1,
            copy_text: None,
            copy_target: None,
        },
    );

    assert!(
        popover_is_open(cx, &view),
        "expected the diff editor context menu to open"
    );
    assert!(
        !diff_panel_is_focused(cx, &view),
        "expected the diff editor context menu to take focus"
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.context_menu_activate_action(
                    ContextMenuAction::ApplyIndexPatch {
                        repo_id,
                        patch: "diff --git a/src/first.rs b/src/first.rs\n".into(),
                        reverse: false,
                    },
                    window,
                    cx,
                );
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    assert!(
        !popover_is_open(cx, &view),
        "expected staging from the diff editor context menu to close the menu"
    );
    assert!(
        diff_panel_is_focused(cx, &view),
        "expected staging from the diff editor context menu to restore diff-panel focus"
    );

    cx.simulate_keystrokes("f4");
    draw_and_drain_test_window(cx);
    wait_until_store_diff_target_path(cx, &view, second.as_path());
    sync_store_snapshot(cx, &view);

    assert_eq!(
        active_worktree_diff_target_path(cx, &view),
        Some(second),
        "expected F4 to navigate immediately after staging from the diff editor context menu"
    );
}

#[gpui::test]
fn space_asks_before_staging_a_file_with_conflict_markers(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70613);
    let commit_id = CommitId("abcdef00112233dd".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_stage_conflict_markers",
        std::process::id()
    ));
    let conflicted = std::path::PathBuf::from("conflicted.rs");
    std::fs::create_dir_all(&workdir).unwrap();
    // Multi-megabyte, with the conflict spanning nearly the whole file: sizing a
    // file out of the scan used to skip the warning entirely.
    let mut content = String::from("a\n<<<<<<< HEAD\n");
    for i in 0..120_000 {
        content.push_str(&format!("ours {i}\n"));
    }
    content.push_str("=======\n");
    for i in 0..120_000 {
        content.push_str(&format!("theirs {i}\n"));
    }
    content.push_str(">>>>>>> other\nb\n");
    assert!(content.len() > 2 * 1024 * 1024);
    std::fs::write(workdir.join(&conflicted), &content).unwrap();

    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&conflicted),
        &conflicted,
    );
    repo.status = Loadable::Ready(
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

    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_diff_panel(cx, &view);
    cx.simulate_keystrokes("space");
    draw_and_drain_test_window(cx);

    let kind =
        cx.update(|_window, app| crate::view::test_support::popover_kind(view.read(app), app));
    assert!(
        matches!(
            kind,
            Some(PopoverKind::StageConflictMarkersConfirm { ref unresolved, .. })
                if unresolved == &vec![conflicted.clone()]
        ),
        "expected the unresolved-conflict confirmation, got {kind:?}"
    );

    // The stage itself must wait for the user's answer.
    assert!(
        cx.update(|_window, app| {
            let snapshot = view.read(app).store.snapshot();
            snapshot
                .repos
                .iter()
                .find(|repo| repo.id == repo_id)
                .is_some_and(|repo| repo.local_actions_in_flight == 0)
        }),
        "nothing may be staged until the confirmation is answered"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
fn space_stages_a_resolved_conflict_without_asking(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70614);
    let commit_id = CommitId("abcdef00112233ee".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_stage_resolved_conflict",
        std::process::id()
    ));
    let resolved = std::path::PathBuf::from("resolved.rs");
    std::fs::create_dir_all(&workdir).unwrap();
    std::fs::write(workdir.join(&resolved), "a\nours\nb\n").unwrap();

    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&resolved),
        &resolved,
    );
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: std::sync::Arc::new(vec![]),
            unstaged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                path: resolved.clone(),
                kind: gitcomet_core::domain::FileStatusKind::Modified,
                conflict: Some(gitcomet_core::domain::FileConflictKind::BothModified),
            }]),
        }
        .into(),
    );

    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_diff_panel(cx, &view);
    cx.simulate_keystrokes("space");
    draw_and_drain_test_window(cx);

    assert!(
        cx.update(|_window, app| crate::view::test_support::popover_kind(view.read(app), app))
            .is_none(),
        "a conflict whose markers are gone must stage without a prompt"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
fn space_keeps_the_diff_when_the_selected_files_cannot_be_staged(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70612);
    let commit_id = CommitId("abcdef00112233cc".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_space_multi_select",
        std::process::id()
    ));
    let first = std::path::PathBuf::from("src/first.rs");
    let second = std::path::PathBuf::from("src/second.rs");
    let third = std::path::PathBuf::from("src/third.rs");
    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        &[first.clone(), second.clone(), third.clone()],
        &first,
    );

    apply_state(cx, &view, app_state_with_active_repo(repo));

    // Stands in for ctrl-clicking the first two rows.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.status_multi_selection.insert(
                    repo_id,
                    StatusMultiSelection {
                        unstaged: vec![first.clone(), second.clone()],
                        unstaged_anchor: Some(first.clone()),
                        ..Default::default()
                    },
                );
                cx.notify();
            });
        });
    });

    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_diff_panel(cx, &view);
    cx.simulate_keystrokes("space");
    draw_and_drain_test_window(cx);

    assert!(
        cx.update(|_window, app| {
            !view
                .read(app)
                .details_pane
                .read(app)
                .status_multi_selection
                .contains_key(&repo_id)
        }),
        "staging the selection must consume it"
    );

    // This fixture has no repository handle, so staging fails. Consuming the
    // selection must not close the diff or advance it to another file.
    wait_until(cx, "the staging failure", |cx| {
        cx.update(|_window, app| {
            let snapshot = view.read(app).store.snapshot();
            snapshot
                .repos
                .iter()
                .find(|repo| repo.id == repo_id)
                .is_some_and(|repo| {
                    repo.local_actions_in_flight == 0 && repo.feedback.last_error.is_some()
                })
        })
    });
    sync_store_snapshot(cx, &view);
    assert_eq!(active_worktree_diff_target_path(cx, &view), Some(first));
}

/// Ctrl+S must resolve the multi-file selection before confirming, the way
/// space does. Confirming on the shown file first makes the dialog describe —
/// and then stage — one file out of a selection of three, leaving the rest
/// unstaged and the selection stranded.
#[gpui::test]
fn ctrl_s_confirms_for_the_whole_ctrl_selected_set(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70614);
    let commit_id = CommitId("abcdef00112233ee".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_ctrl_s_multi_select_conflict",
        std::process::id()
    ));
    let conflicted = std::path::PathBuf::from("conflicted.rs");
    let second = std::path::PathBuf::from("src/second.rs");
    let third = std::path::PathBuf::from("src/third.rs");
    std::fs::create_dir_all(&workdir).unwrap();
    std::fs::write(
        workdir.join(&conflicted),
        "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> other\nb\n",
    )
    .unwrap();

    // The conflicted file is the one the diff pane is showing, so the buggy
    // order confirms on it alone.
    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        &[conflicted.clone(), second.clone(), third.clone()],
        &conflicted,
    );
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: std::sync::Arc::new(vec![]),
            unstaged: std::sync::Arc::new(vec![
                gitcomet_core::domain::FileStatus {
                    path: conflicted.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Modified,
                    conflict: Some(gitcomet_core::domain::FileConflictKind::BothModified),
                },
                gitcomet_core::domain::FileStatus {
                    path: second.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Modified,
                    conflict: None,
                },
                gitcomet_core::domain::FileStatus {
                    path: third.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Modified,
                    conflict: None,
                },
            ]),
        }
        .into(),
    );

    apply_state(cx, &view, app_state_with_active_repo(repo));

    // Stands in for ctrl-clicking all three rows.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.status_multi_selection.insert(
                    repo_id,
                    StatusMultiSelection {
                        unstaged: vec![conflicted.clone(), second.clone(), third.clone()],
                        unstaged_anchor: Some(conflicted.clone()),
                        ..Default::default()
                    },
                );
                cx.notify();
            });
        });
    });

    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_diff_panel(cx, &view);
    cx.simulate_keystrokes("ctrl-s");
    draw_and_drain_test_window(cx);

    let kind =
        cx.update(|_window, app| crate::view::test_support::popover_kind(view.read(app), app));
    let Some(PopoverKind::StageConflictMarkersConfirm {
        paths, unresolved, ..
    }) = kind
    else {
        panic!("expected the unresolved-conflict confirmation, got {kind:?}");
    };
    assert_eq!(
        paths,
        vec![conflicted.clone(), second.clone(), third.clone()],
        "going ahead must stage the whole selection, not just the shown file"
    );
    assert_eq!(
        unresolved,
        vec![conflicted.clone()],
        "only the file with markers left in it is unresolved"
    );

    // The dialog is still up, and the selection it describes is still the user's:
    // resolving the paths must not have consumed it.
    assert_eq!(
        ctrl_selected_unstaged_paths(cx, &view, repo_id),
        vec![conflicted.clone(), second.clone(), third.clone()],
        "the selection must survive while the confirmation is undecided"
    );

    // Cancelling stages nothing and costs the user nothing: dismissing the
    // dialog is the whole of what "Cancel" does.
    cx.update(|_window, app| {
        let host = view.read(app).popover_host.clone();
        host.update(app, |host, cx| host.close_popover(cx));
    });
    draw_and_drain_test_window(cx);
    assert!(
        cx.update(|_window, app| crate::view::test_support::popover_kind(view.read(app), app))
            .is_none(),
        "the confirmation must be gone"
    );
    assert_eq!(
        ctrl_selected_unstaged_paths(cx, &view, repo_id),
        vec![conflicted.clone(), second.clone(), third.clone()],
        "cancelling must leave the selection exactly as the user built it"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

/// The paths currently ctrl-selected in the unstaged list.
fn ctrl_selected_unstaged_paths(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    repo_id: RepoId,
) -> Vec<std::path::PathBuf> {
    cx.update(|_window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .status_multi_selection
            .get(&repo_id)
            .map(|selection| selection.unstaged.clone())
            .unwrap_or_default()
    })
}

#[gpui::test]
fn detached_window_focus_space_stages_and_advances_diff(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70564);
    let commit_id = CommitId("abcdef0011223388".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_detached_focus_space",
        std::process::id()
    ));
    let first = std::path::PathBuf::from("src/first.rs");
    let second = std::path::PathBuf::from("src/second.rs");
    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        &[first.clone(), second.clone()],
        &first,
    );

    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_detached_window_focus(cx);

    cx.simulate_keystrokes("space");
    draw_and_drain_test_window(cx);
    wait_until_store_diff_target_path(cx, &view, second.as_path());
    sync_store_snapshot(cx, &view);

    assert_eq!(
        active_worktree_diff_target_path(cx, &view),
        Some(second),
        "expected Space from detached focus to stage the active file and advance the diff target"
    );
}

#[gpui::test]
fn detached_window_focus_conflict_quick_pick_uses_global_diff_shortcut_fallback(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70565);
    let commit_id = CommitId("abcdef0011223399".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_detached_focus_conflict_pick",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/conflicted.rs");
    let repo = simple_conflict_repo(repo_id, &workdir, &commit_id, path.as_path());

    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    wait_for_main_pane_condition(
        cx,
        &view,
        "conflict resolver state for detached-focus quick pick",
        |pane| {
            pane.conflict_resolver.path.as_deref() == Some(path.as_path())
                && pane
                    .conflict_resolver
                    .resolved_outline
                    .markers
                    .iter()
                    .flatten()
                    .map(|marker| marker.conflict_ix)
                    .max()
                    .is_some_and(|ix| ix >= 1)
        },
        |pane| {
            format!(
                "path={:?} markers={} active_conflict={:?}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.resolved_outline.markers.len(),
                pane.conflict_resolver.active_conflict,
            )
        },
    );

    focus_detached_window_focus(cx);
    cx.simulate_keystrokes("b");
    draw_and_drain_test_window(cx);

    assert_eq!(
        active_conflict_ix(cx, &view),
        1,
        "expected conflict quick-pick key from detached focus to pick the first conflict and advance"
    );
}
