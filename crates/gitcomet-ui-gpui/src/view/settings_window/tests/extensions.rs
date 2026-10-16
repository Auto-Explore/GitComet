use super::*;

#[gpui::test]
fn settings_snapshot_uses_the_shared_renderer_without_a_repository_window(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_extension_api::{DiffLayout, DiffPaneOptions, DiffSnapshot, Registry};
    cx.update(|app| {
        crate::view::extension_host::install(
            Registry::build(vec![Box::new(
                gitcomet_extension_example::review::ReviewExtension,
            )])
            .unwrap(),
            app,
        );
    });
    let (settings, settings_cx) = cx.add_window_view(SettingsWindowView::new);
    settings_cx.run_until_parked();
    let pane = settings_cx.update(|_, app| {
        let host = settings.read(app).extension_window.as_ref().unwrap().host();
        let state = host.state(app).unwrap();
        assert!(state.repos.is_empty());
        assert!(state.active_repo.is_none());
        host.create_snapshot_pane(
            DiffSnapshot::new("rootless.rs", "before\n", "after\n"),
            DiffPaneOptions {
                layout: DiffLayout::Inline,
                ..Default::default()
            },
            app,
        )
        .unwrap()
    });
    settings_cx.run_until_parked();
    struct SnapshotWindow(gpui::AnyView);
    impl Render for SnapshotWindow {
        fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
            div().size_full().child(self.0.clone())
        }
    }
    let snapshot = pane.view();
    let (_, cx) = cx.add_window_view(move |_, _| SnapshotWindow(snapshot));
    for _ in 0..3 {
        crate::view::test_support::redraw(cx);
        cx.run_until_parked();
    }
    cx.update(|_, app| {
        assert_eq!(app.windows().len(), 2);
        assert!(
            app.windows()
                .iter()
                .all(|window| window.downcast::<GitCometView>().is_none())
        );
        assert!(!pane.is_loading(app));
        pane.set_search("after", app);
        assert_eq!(pane.search_matches(app), 1);
        let hosted = pane
            .view()
            .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
            .unwrap();
        let renderer = hosted.read(app).renderer().unwrap().read(app);
        assert!(renderer.active_repo().is_none());
        assert!(renderer.editor_documents.all().is_empty());
        assert!(!renderer.store.policy.allow_edit);
        assert!(!renderer.store.policy.allow_stage);
    });
    assert!(cx.debug_bounds("diff_bar_stage").is_none());
    assert!(cx.debug_bounds("diff_file_edit").is_none());
    assert!(cx.debug_bounds("diff_body_container").is_some());
}

#[gpui::test]
fn settings_pages_are_listed_and_built_only_when_selected(cx: &mut gpui::TestAppContext) {
    cx.update(|app| {
        let registry = gitcomet_extension_api::Registry::build(vec![Box::new(
            gitcomet_extension_example::review::ReviewExtension,
        )])
        .expect("valid registration");
        crate::view::extension_host::install(registry, app);
    });
    let (_settings, cx) = cx.add_window_view(SettingsWindowView::new);
    crate::view::test_support::redraw(cx);
    let nav = "settings_window_nav_extension_com.example.review/review-settings";
    assert!(cx.debug_bounds(nav).is_some(), "the page is listed");
    assert!(
        cx.debug_bounds("example_review_settings").is_none(),
        "an unselected page is not built"
    );

    let center = cx.debug_bounds(nav).unwrap().center();
    cx.simulate_click(center, gpui::Modifiers::default());
    crate::view::test_support::redraw(cx);
    assert!(cx.debug_bounds("example_review_settings").is_some());

    let general = cx
        .debug_bounds("settings_window_nav_general")
        .unwrap()
        .center();
    cx.simulate_click(general, gpui::Modifiers::default());
    crate::view::test_support::redraw(cx);
    assert!(cx.debug_bounds("example_review_settings").is_none());
    assert!(cx.debug_bounds("settings_window_general").is_some());
}

/// A settings page gets the window's host: its reset opens a hosted dialog,
/// confirming closes it, and its toasts keep their kind.
#[gpui::test]
fn settings_pages_open_dialogs_and_toasts_through_their_host(cx: &mut gpui::TestAppContext) {
    cx.update(|app| {
        let registry = gitcomet_extension_api::Registry::build(vec![Box::new(
            gitcomet_extension_example::review::ReviewExtension,
        )])
        .expect("valid registration");
        crate::view::extension_host::install(registry, app);
    });
    let (settings, cx) = cx.add_window_view(SettingsWindowView::new);
    crate::view::test_support::redraw(cx);
    let nav = "settings_window_nav_extension_com.example.review/review-settings";
    let center = cx.debug_bounds(nav).unwrap().center();
    cx.simulate_click(center, gpui::Modifiers::default());
    crate::view::test_support::redraw(cx);

    let reset = cx.debug_bounds("example_review_reset").unwrap().center();
    cx.simulate_click(reset, gpui::Modifiers::default());
    cx.run_until_parked();
    crate::view::test_support::redraw(cx);
    assert!(cx.debug_bounds("settings_hosted_dialog").is_some());
    assert!(cx.debug_bounds("example_reset_confirm").is_some());

    let confirm = cx
        .debug_bounds("example_reset_confirm_button")
        .unwrap()
        .center();
    cx.simulate_click(confirm, gpui::Modifiers::default());
    cx.run_until_parked();
    crate::view::test_support::redraw(cx);
    assert!(cx.debug_bounds("settings_hosted_dialog").is_none());
    assert!(cx.debug_bounds("settings_notice").is_some());

    let host = cx.update(|_, app| settings.read(app).extension_window.as_ref().unwrap().host());
    cx.update(|_, app| {
        host.report_error("Could not reach the server", Vec::new(), app)
            .unwrap()
    });
    cx.run_until_parked();
    crate::view::test_support::redraw(cx);
    cx.update(|_, app| {
        let (kind, message, _) = settings.read(app).extension_notice.clone().unwrap();
        assert_eq!(kind, gitcomet_extension_api::NotificationKind::Error);
        assert_eq!(message.as_ref(), "Could not reach the server");
    });
}

#[gpui::test]
fn settings_extensions_have_a_host_revisioned_gates_and_window_lifetime(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_extension_api::*;
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };
    struct Instance(Rc<Cell<usize>>);
    impl WindowExtension for Instance {}
    impl Drop for Instance {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    struct ExtensionProbe {
        host: Rc<RefCell<Option<WindowHost>>>,
        drops: Rc<Cell<usize>>,
        active: Rc<Cell<bool>>,
        calls: Rc<Cell<usize>>,
        signal: SlotSignal,
    }
    impl Extension for ExtensionProbe {
        fn id(&self) -> ExtensionId {
            ExtensionId::new("com.example.settings-test").unwrap()
        }
        fn register(&self, r: &mut Registrar) {
            let active = self.active.clone();
            let calls = self.calls.clone();
            r.window_gate(
                "gate",
                WindowGateDescriptor::new(
                    self.signal.clone(),
                    move |host, _| {
                        assert_eq!(host.kind(), gitcomet_core::identity::WindowKind::Settings);
                        calls.set(calls.get() + 1);
                        active.get()
                    },
                    |_, _, cx| cx.new(|_| gpui::Empty).into(),
                ),
            );
        }
        fn window_opened(
            &self,
            host: WindowHost,
            _: &mut Window,
            _: &mut App,
        ) -> Option<Box<dyn WindowExtension>> {
            *self.host.borrow_mut() = Some(host);
            Some(Box::new(Instance(self.drops.clone())))
        }
    }
    let host = Rc::new(RefCell::new(None));
    let drops = Rc::new(Cell::new(0));
    let active = Rc::new(Cell::new(true));
    let calls = Rc::new(Cell::new(0));
    let signal = SlotSignal::default();
    cx.update(|app| {
        crate::view::extension_host::install(
            Registry::build(vec![Box::new(ExtensionProbe {
                host: host.clone(),
                drops: drops.clone(),
                active: active.clone(),
                calls: calls.clone(),
                signal: signal.clone(),
            })])
            .unwrap(),
            app,
        )
    });
    let (_, cx) = cx.add_window_view(SettingsWindowView::new);
    cx.run_until_parked();
    let host = host.borrow().clone().expect("settings window host");
    for _ in 0..3 {
        crate::view::test_support::redraw(cx);
    }
    assert_eq!(calls.get(), 1, "unchanged gates are not reevaluated");
    assert!(cx.debug_bounds("settings_window_general").is_none());
    active.set(false);
    signal.bump();
    host.notifier().notify(Slot::Gate);
    cx.run_until_parked();
    crate::view::test_support::redraw(cx);
    assert!(cx.debug_bounds("settings_window_general").is_some());
    assert_eq!(calls.get(), 2);
    cx.update(|window, _| window.remove_window());
    cx.run_until_parked();
    assert_eq!(drops.get(), 1);
    cx.cx.update(|app| assert!(!host.is_open(app)));
}
