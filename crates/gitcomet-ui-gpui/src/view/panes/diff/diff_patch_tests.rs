use crate::view::*;

#[gpui::test]
fn structural_context_menu_opens_without_installed_extensions(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(crate::view::test_support::TestBackend));
    let (root, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        assert!(root.read(app).extension_window.is_none());
        root.read(app)
            .diff_renderer
            .clone()
            .update(app, |pane, cx| {
                pane.open_structural_menu(
                    Vec::new(),
                    Some("selected text".into()),
                    point(px(100.0), px(100.0)),
                    window,
                    cx,
                );
            });
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        assert!(crate::view::test_support::popover_is_open(
            root.read(app),
            app
        ));
    });
}
