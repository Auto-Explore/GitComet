use super::*;
use gitcomet_extension_api::{DiffLayout, Registry};
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
            diff_text: "diff --git a/example.rs b/example.rs\n--- a/example.rs\n+++ b/example.rs\n@@ -1,2 +1,2 @@\n first\n-old\n+new\n".into(),
        }),
        workspace: WorkspaceBootstrap::Empty,
        ..Default::default()
    }, window, cx))
}

#[gpui::test]
fn focused_diff_uses_the_common_pane_and_gate_at_each_width(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
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
        let pane = view.focused_diff_pane.clone().unwrap();
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
    let _guard = crate::test_support::lock_visual_test();
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
    let _guard = crate::test_support::lock_visual_test();
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
    let _guard = crate::test_support::lock_visual_test();
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
    let _guard = crate::test_support::lock_visual_test();
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
    let (host, main_pane) = cx.update(|_, app| {
        let view = view.read(app);
        (
            view.extension_window.as_ref().unwrap().host(),
            view.main_pane.clone(),
        )
    });
    cx.update(|window, app| {
        main_pane.update(app, |pane, cx| {
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
