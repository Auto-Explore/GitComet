use super::*;
use gitcomet_extension_api::{DiffLayout, DiffLineSide, Registry};
use std::sync::atomic::{AtomicI32, Ordering};

/// A focused mergetool window on a repository that need not exist; its exit
/// code lands in `exit_code`.
fn mergetool(
    cx: &mut gpui::TestAppContext,
    exit_code: Arc<AtomicI32>,
) -> (Entity<GitCometView>, &mut gpui::VisualTestContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    store.replace_snapshot_for_test(Arc::new(AppState {
        git_runtime: available_git_runtime_state(),
        ..AppState::test_default()
    }));
    let config = GitCometViewConfig {
        view_mode: GitCometViewMode::FocusedMergetool,
        focused_mergetool: Some(FocusedMergetoolViewConfig {
            repo_path: PathBuf::from("/tmp/focused-mergetool-host-repo"),
            conflicted_file_path: PathBuf::from("conflicted.txt"),
            labels: FocusedMergetoolLabels {
                local: "LOCAL".to_string(),
                remote: "REMOTE".to_string(),
                base: "BASE".to_string(),
            },
        }),
        focused_mergetool_exit_code: Some(exit_code),
        ..GitCometViewConfig::default()
    };
    cx.add_window_view(|window, cx| {
        GitCometView::new_with_config(store, events, config, window, cx)
    })
}

fn install_example_registry(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        crate::view::extension_host::install_registry(
            Registry::build(vec![Box::new(
                gitcomet_extension_example::review::ReviewExtension,
            )])
            .unwrap(),
            cx,
        )
    });
}

fn focused(
    cx: &mut gpui::TestAppContext,
    available: bool,
) -> (Entity<GitCometView>, &mut gpui::VisualTestContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    store.replace_snapshot_for_test(Arc::new(AppState {
        git_runtime: if available {
            available_git_runtime_state()
        } else {
            unavailable_git_runtime_state()
        },
        ..AppState::test_default()
    }));
    cx.add_window_view(|window, cx| GitCometView::new_with_config(store, events, GitCometViewConfig {
        view_mode: GitCometViewMode::FocusedDiff,
        focused_diff: Some(crate::FocusedDiffConfig {
            label_left: "before".into(), label_right: "after".into(), display_path: Some("example.rs".into()),
            input: crate::FocusedDiffInput::Patch("diff --git a/example.rs b/example.rs\n--- a/example.rs\n+++ b/example.rs\n@@ -1,2 +1,2 @@\n first\n-old\n+new\n".into()),
        }),
        workspace: WorkspaceBootstrap::Empty,
        ..Default::default()
    }, window, cx))
}

#[gpui::test]
fn structural_fallback_notice_remains_visible_in_narrow_focused_panes(
    cx: &mut gpui::TestAppContext,
) {
    let (root, cx) = focused(cx, true);
    cx.update(|_, app| {
        let pane = root
            .read(app)
            .focused_diff_pane
            .as_ref()
            .unwrap()
            .pane
            .clone();
        pane.set_engine_override(Some(DiffEngine::Difftastic), app);
    });
    cx.run_until_parked();
    for width in [640.0, 400.0] {
        cx.simulate_resize(gpui::size(px(width), px(800.0)));
        test_support::redraw(cx);
        let notice = cx.debug_bounds("diff_engine_notice").unwrap();
        let label = cx.debug_bounds("diff_engine_notice_label").unwrap();
        let header = cx.debug_bounds("diff_file_header").unwrap();
        assert!(notice.top() >= header.bottom());
        assert!(label.left() >= notice.left());
        assert!(label.right() <= notice.right());
        assert!(label.size.height > px(0.0));
    }
}

#[gpui::test]
fn focused_patch_fallback_paints_both_sides_in_each_layout(cx: &mut gpui::TestAppContext) {
    let (root, cx) = focused(cx, true);
    let pane = cx.update(|_, app| {
        root.read(app)
            .focused_diff_pane
            .as_ref()
            .unwrap()
            .pane
            .clone()
    });
    cx.update(|_, app| pane.set_engine_override(Some(DiffEngine::Difftastic), app));
    for layout in [DiffLayout::Split, DiffLayout::Inline] {
        cx.update(|_, app| pane.set_layout(layout, app));
        test_support::redraw(cx);
        pump_until(cx, "standalone patch rows painted", |cx| {
            cx.update(|_, app| {
                let hosted = pane
                    .view()
                    .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
                    .unwrap();
                let Some(renderer) = hosted.read(app).renderer() else {
                    return false;
                };
                let renderer = renderer.read(app);
                ["old", "new"].into_iter().all(|text| {
                    renderer
                        .diff_text_hitboxes
                        .values()
                        .any(|hitbox| hitbox.painted_text.contains(text))
                })
            })
        });
        assert!(cx.debug_bounds("diff_engine_notice").is_some());
    }
}

#[gpui::test]
fn focused_file_pair_retains_context_outside_patch_and_uses_native_layouts(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let text = "outside patch\n1\n2\n3\n4\n5\nold\n";
    let (view, cx) = cx.add_window_view(|window, cx| {
        GitCometView::new_with_config(
            store,
            events,
            GitCometViewConfig {
                view_mode: GitCometViewMode::FocusedDiff,
                focused_diff: Some(crate::FocusedDiffConfig {
                    label_left: "before".into(),
                    label_right: "after".into(),
                    display_path: Some("file.rs".into()),
                    input: crate::FocusedDiffInput::FilePair {
                        path: "file.rs".into(),
                        old: text.into(),
                        new: text.replace("old", "new").into(),
                    },
                }),
                workspace: WorkspaceBootstrap::Empty,
                ..Default::default()
            },
            window,
            cx,
        )
    });
    cx.run_until_parked();
    for _ in 0..3 {
        test_support::redraw(cx);
    }
    let pane = cx.update(|_, app| view.read(app).focused_diff_pane.clone().unwrap().pane);
    assert!(cx.debug_bounds("focused_diff_left_label").is_some());
    assert!(cx.debug_bounds("focused_diff_right_label").is_some());
    cx.update(|_, app| {
        assert_eq!(
            view.read(app).focused_diff_pane.as_ref().unwrap().labels,
            [SharedString::from("before"), SharedString::from("after")]
        )
    });
    for layout in [DiffLayout::Split, DiffLayout::Inline] {
        cx.update(|_, app| {
            pane.set_layout(layout, app);
            pane.set_search("outside patch", app);
        });
        cx.run_until_parked();
        test_support::redraw(cx);
        cx.update(|_, app| {
            assert!(!pane.is_loading(app));
            assert!(pane.search_matches(app) > 0);
            let hosted = pane
                .view()
                .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
                .unwrap();
            let renderer = hosted.read(app).renderer().unwrap().read(app);
            assert_eq!(
                renderer
                    .file_diff_split_row(0)
                    .unwrap()
                    .old
                    .unwrap()
                    .as_ref(),
                "outside patch"
            );
            assert!(!renderer.store.policy.allow_stage);
            assert!(!renderer.store.policy.allow_edit);
        });
    }
}

/// A focused diff window over `input`, shown as `notes.txt`.
fn focused_over(
    cx: &mut gpui::TestAppContext,
    input: crate::FocusedDiffInput,
) -> (Entity<GitCometView>, &mut gpui::VisualTestContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    cx.add_window_view(|window, cx| {
        GitCometView::new_with_config(
            store,
            events,
            GitCometViewConfig {
                view_mode: GitCometViewMode::FocusedDiff,
                focused_diff: Some(crate::FocusedDiffConfig {
                    label_left: "before".into(),
                    label_right: "after".into(),
                    display_path: Some("notes.txt".into()),
                    input,
                }),
                workspace: WorkspaceBootstrap::Empty,
                ..Default::default()
            },
            window,
            cx,
        )
    })
}

fn hosted_pane(
    view: &Entity<GitCometView>,
    app: &App,
) -> Entity<crate::view::hosted::diff_pane::DiffPaneView> {
    view.read(app)
        .focused_diff_pane
        .as_ref()
        .unwrap()
        .pane
        .view()
        .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
        .unwrap()
}

/// The status each frame of a focused diff over `input` drew, once its rows
/// are in. Each frame's own: a later test frame would hide a stale one.
fn statuses_while_rows_arrive(
    cx: &mut gpui::TestAppContext,
    input: crate::FocusedDiffInput,
) -> Vec<Option<SharedString>> {
    let (view, cx) = focused_over(cx, input);
    cx.run_until_parked();
    cx.update(|_, app| {
        let entity = hosted_pane(&view, app);
        let entity = entity.read(app);
        assert!(entity.renderer().unwrap().read(app).diff_visible_len() > 0);
        entity.rendered_statuses().to_vec()
    })
}

/// The standalone difftool said "No changes" above a full diff: the pane read
/// the renderer's row count before the renderer refreshed it in that frame.
/// The renderer shows its own loading and empty states, so the pane adds none.
#[gpui::test]
fn focused_file_pair_never_says_no_changes_while_its_rows_arrive(cx: &mut gpui::TestAppContext) {
    let statuses = statuses_while_rows_arrive(
        cx,
        crate::FocusedDiffInput::FilePair {
            path: "notes.txt".into(),
            old: "first\nold\n".into(),
            new: "first\nnew\n".into(),
        },
    );
    assert!(statuses.iter().all(Option::is_none), "{statuses:?}");
}

/// Patch input (directories, mode changes) builds its rows when the renderer
/// is created rather than when a file diff lands; its first frame went stale.
#[gpui::test]
fn focused_patch_never_says_no_changes_while_its_rows_arrive(cx: &mut gpui::TestAppContext) {
    let statuses = statuses_while_rows_arrive(
        cx,
        crate::FocusedDiffInput::Patch(
            "diff --git a/notes.txt b/notes.txt\n--- a/notes.txt\n+++ b/notes.txt\n@@ -1,2 +1,2 @@\n first\n-old\n+new\n".into(),
        ),
    );
    assert!(statuses.iter().all(Option::is_none), "{statuses:?}");
}

/// 400 lines that each wrap into several rows; line 300 differs.
fn wrapped_pair() -> crate::FocusedDiffInput {
    let pad = "x".repeat(400);
    let old: String = (1..=400).map(|i| format!("line {i} {pad}\n")).collect();
    let new = old.replace(
        &format!("line 300 {pad}\n"),
        &format!("line 300 changed {pad}\n"),
    );
    crate::FocusedDiffInput::FilePair {
        path: "notes.txt".into(),
        old: old.into(),
        new: new.into(),
    }
}

/// The `line N` labels the pane painted, with the wrapped rows built.
fn painted_lines(view: &Entity<GitCometView>, cx: &mut gpui::VisualTestContext) -> Vec<String> {
    cx.update(|_, app| {
        let entity = hosted_pane(view, app);
        let renderer = entity.read(app).renderer().unwrap().read(app);
        assert!(renderer.diff_wrap_visible_rows.len() > 400);
        let mut lines: Vec<String> = renderer
            .diff_text_hitboxes
            .values()
            .filter_map(|hitbox| {
                let mut words = hitbox.painted_text.split(' ');
                (words.next() == Some("line"))
                    .then(|| format!("line {}", words.next().unwrap_or("")))
            })
            .collect();
        lines.sort();
        lines.dedup();
        lines
    })
}

fn wrap_and_reveal(view: &Entity<GitCometView>, line: u32, cx: &mut gpui::VisualTestContext) {
    cx.update(|_, app| {
        let renderer = hosted_pane(view, app).read(app).renderer().unwrap().clone();
        renderer.update(app, |pane, cx| pane.set_diff_word_wrap(true, cx));
        let pane = view.read(app).focused_diff_pane.clone().unwrap().pane;
        pane.reveal(DiffLineSide::New, line, app);
    });
}

/// A reveal asked for before a word-wrapped pane's rows arrive must land on
/// its line. Its row was a source row, applied to the wrapped list once built.
#[gpui::test]
fn a_reveal_before_wrapped_rows_arrive_lands_on_its_line(cx: &mut gpui::TestAppContext) {
    let (view, cx) = focused_over(cx, wrapped_pair());
    wrap_and_reveal(&view, 200, cx);
    cx.run_until_parked();
    let painted = painted_lines(&view, cx);
    assert!(painted.iter().any(|line| line == "line 200"), "{painted:?}");
}

/// Switching engines keeps a word-wrapped pane on the line it showed.
#[gpui::test]
fn an_engine_switch_keeps_a_wrapped_pane_on_its_line(cx: &mut gpui::TestAppContext) {
    let (view, cx) = focused_over(cx, wrapped_pair());
    cx.run_until_parked();
    wrap_and_reveal(&view, 200, cx);
    cx.run_until_parked();
    let before = painted_lines(&view, cx);
    assert!(before.iter().any(|line| line == "line 200"), "{before:?}");
    cx.update(|_, app| {
        let pane = view.read(app).focused_diff_pane.clone().unwrap().pane;
        pane.set_engine_override(Some(DiffEngine::Difftastic), app);
    });
    cx.run_until_parked();
    let after = painted_lines(&view, cx);
    assert!(after.iter().any(|line| line == "line 200"), "{after:?}");
}

#[gpui::test]
fn focused_diff_uses_the_common_pane_and_gate_at_each_width(cx: &mut gpui::TestAppContext) {
    let (view, cx) = focused(cx, false);
    cx.run_until_parked();
    test_support::redraw(cx);
    assert!(cx.debug_bounds("git_unavailable_screen").is_some());
    let (pane, id) = cx.update(|_, app| {
        let view = view.read(app);
        let host = view.extension_window.as_ref().unwrap().host();
        assert_eq!(
            host.kind(),
            gitcomet_core::identity::WindowKind::FocusedDiff
        );
        let pane = view.focused_diff_pane.clone().unwrap().pane;
        let entity = pane
            .view()
            .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
            .unwrap();
        (pane, entity.read(app).view_id())
    });
    let selector: &'static str = Box::leak(format!("hosted_diff_{id}").into_boxed_str());
    assert!(cx.debug_bounds(selector).is_none());
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            test_support::push_test_state(
                view,
                Arc::new(AppState {
                    git_runtime: available_git_runtime_state(),
                    ..AppState::test_default()
                }),
                cx,
            )
        })
    });
    for width in [840.0, 1160.0, 1440.0] {
        cx.simulate_resize(gpui::size(px(width), px(800.0)));
        for layout in [DiffLayout::Inline, DiffLayout::Split] {
            cx.update(|_, app| pane.set_layout(layout, app));
            cx.run_until_parked();
            for _ in 0..3 {
                test_support::redraw(cx);
            }
            let bounds = cx.debug_bounds(selector).expect("focused pane");
            assert!(
                f32::from(bounds.size.width) >= width - 32.0,
                "{bounds:?} at {width}"
            );
            assert!(f32::from(bounds.size.width) <= width);
            assert!(cx.debug_bounds("git_unavailable_screen").is_none());
        }
    }
}

#[gpui::test]
fn focused_diff_escape_and_q_close_the_window(cx: &mut gpui::TestAppContext) {
    for key in ["escape", "q"] {
        let (view, cx) = focused(cx, true);
        cx.run_until_parked();
        for _ in 0..3 {
            test_support::redraw(cx);
        }
        let host = cx.update(|window, app| {
            window.activate();
            view.read(app).extension_window.as_ref().unwrap().host()
        });
        cx.simulate_click(
            gpui::point(px(350.0), px(200.0)),
            gpui::Modifiers::default(),
        );
        cx.simulate_keystrokes(key);
        cx.run_until_parked();
        cx.cx
            .update(|app| assert!(!host.is_open(app), "{key} closes the focused diff"));
    }
}

#[gpui::test]
fn focused_diff_close_runs_extension_guards(cx: &mut gpui::TestAppContext) {
    use gitcomet_extension_api::*;
    struct Guard;
    impl Extension for Guard {
        fn id(&self) -> ExtensionId {
            ExtensionId::new("com.example.focused-guard").unwrap()
        }
        fn register(&self, r: &mut Registrar) {
            r.close_guard(
                "pending",
                std::rc::Rc::new(|request, _| {
                    assert_eq!(request.scope, CloseScope::Window);
                    assert_eq!(
                        request.window.kind(),
                        gitcomet_core::identity::WindowKind::FocusedDiff
                    );
                    CloseDecision::Confirm {
                        reason: "An annotation is unsaved.".into(),
                    }
                }),
            );
        }
    }
    cx.update(|cx| {
        cx.bind_keys([gpui::KeyBinding::new(
            "escape",
            PopoverPromptDismiss,
            Some("PopoverPrompt"),
        )]);
        // What a focused launch installs: the registry without commands.
        crate::view::extension_host::install_registry(
            Registry::build(vec![Box::new(Guard)]).unwrap(),
            cx,
        )
    });
    let (view, cx) = focused(cx, true);
    cx.run_until_parked();
    for _ in 0..3 {
        test_support::redraw(cx);
    }
    cx.simulate_click(
        gpui::point(px(350.0), px(200.0)),
        gpui::Modifiers::default(),
    );
    cx.simulate_keystrokes("q");
    cx.run_until_parked();
    test_support::redraw(cx);
    assert!(cx.debug_bounds("close_guard_reasons").is_some());
    cx.update(|_, app| {
        assert!(
            view.read(app)
                .extension_window
                .as_ref()
                .unwrap()
                .host()
                .is_open(app)
        )
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    test_support::redraw(cx);
    assert!(cx.debug_bounds("close_guard_reasons").is_none());
    cx.update(|_, app| {
        assert!(
            view.read(app)
                .extension_window
                .as_ref()
                .unwrap()
                .host()
                .is_open(app)
        )
    });
}

/// Window gates come first in every root, focused tools included: the
/// windows Git itself opens are gated like the main one.
#[gpui::test]
fn extension_gates_cover_focused_diff_and_mergetool_roots(cx: &mut gpui::TestAppContext) {
    install_example_registry(cx);
    cx.update(|cx| gitcomet_extension_example::review::set_gated(true, cx));
    {
        let (_view, cx) = focused(cx, true);
        cx.run_until_parked();
        test_support::redraw(cx);
        assert!(cx.debug_bounds("example_gate").is_some(), "focused diff");
    }
    let (view, cx) = mergetool(cx, Arc::new(AtomicI32::new(0)));
    cx.run_until_parked();
    test_support::redraw(cx);
    assert!(
        cx.debug_bounds("example_gate").is_some(),
        "focused mergetool"
    );
    cx.update(|_, app| {
        assert_eq!(
            view.read(app)
                .extension_window
                .as_ref()
                .unwrap()
                .host()
                .kind(),
            gitcomet_core::identity::WindowKind::FocusedMergetool
        );
        assert!(
            crate::view::extension_host::palette_entries(app).is_empty(),
            "focused tools offer no extension commands"
        );
    });
}

/// Cancelling the focused mergetool asks the close guards first, like closing
/// its window; confirming exits as cancelled.
#[gpui::test]
fn cancelling_the_focused_mergetool_runs_the_close_guards(cx: &mut gpui::TestAppContext) {
    use gitcomet_extension_api::*;
    struct Guard;
    impl Extension for Guard {
        fn id(&self) -> ExtensionId {
            ExtensionId::new("com.example.mergetool-guard").unwrap()
        }
        fn register(&self, r: &mut Registrar) {
            r.close_guard(
                "pending",
                std::rc::Rc::new(|request, _| {
                    assert_eq!(request.scope, CloseScope::Window);
                    assert_eq!(
                        request.window.kind(),
                        gitcomet_core::identity::WindowKind::FocusedMergetool
                    );
                    CloseDecision::Confirm {
                        reason: "A merge note is unsaved.".into(),
                    }
                }),
            );
        }
    }
    cx.update(|cx| {
        crate::view::extension_host::install_registry(
            Registry::build(vec![Box::new(Guard)]).unwrap(),
            cx,
        )
    });
    let exit_code = Arc::new(AtomicI32::new(-7));
    let (view, cx) = mergetool(cx, Arc::clone(&exit_code));
    cx.run_until_parked();
    test_support::redraw(cx);
    let (host, diff_renderer) = cx.update(|_, app| {
        let view = view.read(app);
        (
            view.extension_window.as_ref().unwrap().host(),
            view.diff_renderer.clone(),
        )
    });
    cx.update(|window, app| {
        diff_renderer.update(app, |pane, cx| {
            pane.close_diff_or_cancel(RepoId(1), window, cx)
        })
    });
    cx.run_until_parked();
    test_support::redraw(cx);
    assert!(cx.debug_bounds("close_guard_reasons").is_some());
    assert_eq!(exit_code.load(Ordering::SeqCst), 1, "cancelled");
    cx.update(|_, app| assert!(host.is_open(app), "the guard holds the window"));

    click_debug_selector(cx, "close_guard_confirm");
    cx.run_until_parked();
    cx.cx
        .update(|app| assert!(!host.is_open(app), "confirming closes it"));
    assert_eq!(exit_code.load(Ordering::SeqCst), 1);
}
