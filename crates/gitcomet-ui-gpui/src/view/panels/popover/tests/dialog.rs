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
