use super::*;

fn open_popover_and_draw(
    view: &gpui::Entity<GitCometView>,
    kind: PopoverKind,
    cx: &mut gpui::VisualTestContext,
) {
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    kind,
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
}

fn assert_popover_open(view: &gpui::Entity<GitCometView>, app: &gpui::App, expected: bool) {
    let is_open = view.read(app).popover_host.read(app).is_open();
    assert_eq!(is_open, expected);
}

// ── Category 1: Esc hint renders on Cancel buttons ──

#[gpui::test]
fn force_push_confirm_renders_cancel_hint(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, mut cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    open_popover_and_draw(
        &view,
        PopoverKind::ForcePushConfirm { repo_id: RepoId(1) },
        &mut cx,
    );
    cx.debug_bounds("force_push_cancel_hint")
        .expect("expected force push Cancel shortcut hint");
}

#[gpui::test]
fn stash_prompt_renders_cancel_hint(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, mut cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    open_popover_and_draw(&view, PopoverKind::StashPrompt, &mut cx);
    cx.debug_bounds("stash_cancel_hint")
        .expect("expected stash Cancel shortcut hint");
}

#[gpui::test]
fn reset_prompt_renders_cancel_hint(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, mut cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    open_popover_and_draw(
        &view,
        PopoverKind::ResetPrompt {
            repo_id: RepoId(1),
            target: "HEAD".to_string(),
            mode: ResetMode::Mixed,
        },
        &mut cx,
    );
    cx.debug_bounds("reset_cancel_hint")
        .expect("expected reset Cancel shortcut hint");
}

// ── Category 2: Esc dismisses popovers ──

#[gpui::test]
fn force_push_confirm_escape_closes(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, mut cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    open_popover_and_draw(
        &view,
        PopoverKind::ForcePushConfirm { repo_id: RepoId(1) },
        &mut cx,
    );
    cx.update(|_window, app| assert_popover_open(&view, app, true));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
        assert_popover_open(&view, app, false);
    });
}

#[gpui::test]
fn stash_prompt_escape_closes(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, mut cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    open_popover_and_draw(&view, PopoverKind::StashPrompt, &mut cx);
    cx.update(|_window, app| assert_popover_open(&view, app, true));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
        assert_popover_open(&view, app, false);
    });
}

#[gpui::test]
fn reset_prompt_escape_closes(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, mut cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    open_popover_and_draw(
        &view,
        PopoverKind::ResetPrompt {
            repo_id: RepoId(1),
            target: "HEAD".to_string(),
            mode: ResetMode::Mixed,
        },
        &mut cx,
    );
    cx.update(|_window, app| assert_popover_open(&view, app, true));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
        assert_popover_open(&view, app, false);
    });
}

#[gpui::test]
fn pull_reconcile_prompt_escape_closes(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, mut cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    open_popover_and_draw(
        &view,
        PopoverKind::PullReconcilePrompt { repo_id: RepoId(1) },
        &mut cx,
    );
    cx.update(|_window, app| assert_popover_open(&view, app, true));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
        assert_popover_open(&view, app, false);
    });
}

#[gpui::test]
fn terminal_shutdown_confirm_escape_closes(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, mut cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    open_popover_and_draw(
        &view,
        PopoverKind::TerminalShutdownConfirm(TerminalShutdownPrompt {
            action: TerminalShutdownAction::CloseWindow,
            summary: TerminalShutdownSummary::default(),
        }),
        &mut cx,
    );
    cx.update(|_window, app| assert_popover_open(&view, app, true));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
        assert_popover_open(&view, app, false);
    });
}

#[gpui::test]
fn discard_changes_confirm_escape_closes(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, mut cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    open_popover_and_draw(
        &view,
        PopoverKind::DiscardChangesConfirm {
            repo_id: RepoId(1),
            area: DiffArea::Unstaged,
            path: None,
        },
        &mut cx,
    );
    cx.update(|_window, app| assert_popover_open(&view, app, true));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
        assert_popover_open(&view, app, false);
    });
}

#[gpui::test]
fn submodule_change_pointer_escape_closes(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, mut cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    open_popover_and_draw(
        &view,
        PopoverKind::Repo {
            repo_id: RepoId(1),
            kind: RepoPopoverKind::Submodule(SubmodulePopoverKind::ChangePointerPrompt {
                path: std::path::PathBuf::from("."),
            }),
        },
        &mut cx,
    );
    cx.update(|_window, app| assert_popover_open(&view, app, true));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
        assert_popover_open(&view, app, false);
    });
}

/// Seed `repo_id` with `path` untracked in the unstaged lane.
///
/// The dialog prefills itself from `add_to_gitignore_target`, which refuses any
/// path it cannot confirm is untracked — so without this the field opens empty.
fn seed_untracked(
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    repo_id: RepoId,
    path: &str,
) {
    let path = std::path::PathBuf::from(path);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: std::env::temp_dir().join("gitcomet_ui_test_gitignore_dialog"),
                },
            );
            repo.status = Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: std::sync::Arc::new(vec![]),
                    unstaged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                        path,
                        kind: gitcomet_core::domain::FileStatusKind::Untracked,
                        conflict: None,
                    }]),
                }
                .into(),
            );
            let state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });
            this.state = Arc::clone(&state);
            this.popover_host.update(cx, |host, _cx| {
                host.state = Arc::clone(&state);
            });
            cx.notify();
        });
    });
}

/// Read the current text of the "Add to .gitignore" pattern field.
fn gitignore_pattern_text(view: &gpui::Entity<GitCometView>, app: &gpui::App) -> String {
    view.read(app)
        .popover_host
        .read(app)
        .gitignore_patterns_input
        .read(app)
        .text()
        .to_string()
}

#[gpui::test]
fn add_to_gitignore_prompt_escape_closes(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, mut cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    open_popover_and_draw(
        &view,
        PopoverKind::AddToGitignorePrompt {
            repo_id: RepoId(1),
            area: DiffArea::Unstaged,
            path: std::path::PathBuf::from("build/out.log"),
        },
        &mut cx,
    );
    cx.update(|_window, app| assert_popover_open(&view, app, true));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
        assert_popover_open(&view, app, false);
    });
}

#[gpui::test]
fn add_to_gitignore_prompt_prefills_the_anchored_file_pattern(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, mut cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    seed_untracked(&view, &mut cx, RepoId(1), "build/out.log");
    open_popover_and_draw(
        &view,
        PopoverKind::AddToGitignorePrompt {
            repo_id: RepoId(1),
            area: DiffArea::Unstaged,
            path: std::path::PathBuf::from("build/out.log"),
        },
        &mut cx,
    );

    cx.update(|_window, app| {
        assert_eq!(
            gitignore_pattern_text(&view, app),
            "/build/out.log",
            "the field opens on the File scope, anchored at the repository root"
        );
    });
}

#[gpui::test]
fn add_to_gitignore_prompt_scope_switch_rewrites_the_pattern(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, mut cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    seed_untracked(&view, &mut cx, RepoId(1), "build/out.log");
    open_popover_and_draw(
        &view,
        PopoverKind::AddToGitignorePrompt {
            repo_id: RepoId(1),
            area: DiffArea::Unstaged,
            path: std::path::PathBuf::from("build/out.log"),
        },
        &mut cx,
    );

    for (scope, expected) in [
        (gitcomet_core::gitignore::GitignoreScope::Folder, "/build/"),
        (gitcomet_core::gitignore::GitignoreScope::Extension, "*.log"),
        (
            gitcomet_core::gitignore::GitignoreScope::File,
            "/build/out.log",
        ),
    ] {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.set_add_to_gitignore_scope(scope, cx);
                });
            });
        });
        cx.update(|_window, app| {
            assert_eq!(gitignore_pattern_text(&view, app), expected);
        });
    }
}

#[gpui::test]
fn add_to_gitignore_prompt_submits_the_hand_edited_pattern(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, mut cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    let path = std::path::PathBuf::from("build/out.log");
    open_popover_and_draw(
        &view,
        PopoverKind::AddToGitignorePrompt {
            repo_id: RepoId(1),
            area: DiffArea::Unstaged,
            path: path.clone(),
        },
        &mut cx,
    );

    // The user trims the suggestion down to the whole directory, and leaves a
    // blank line behind while doing it.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.gitignore_patterns_input.update(cx, |input, cx| {
                    input.set_text("/target/\n\n  *.tmp  ", cx);
                });
            });
        });
    });

    cx.update(|_window, app| {
        let patterns = view
            .read(app)
            .popover_host
            .read(app)
            .add_to_gitignore_patterns(app);
        assert_eq!(
            patterns,
            vec!["/target/".to_string(), "*.tmp".to_string()],
            "blank lines are dropped and each line is trimmed, so a stray edit \
             cannot write an empty or space-padded pattern"
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.submit_add_to_gitignore(RepoId(1), DiffArea::Unstaged, path.clone(), cx);
            });
        });
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
        assert_popover_open(&view, app, false);
    });
}
