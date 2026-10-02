use super::*;
use gitcomet_extension_api::DiffLayout;

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
        crate::view::extension_host::install(Registry::build(vec![Box::new(Guard)]).unwrap(), cx)
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
