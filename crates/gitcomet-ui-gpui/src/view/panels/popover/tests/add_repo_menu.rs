use super::*;

#[gpui::test]
fn add_repo_menu_keyboard_navigation_opens_clone_prompt(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::AddRepoMenu,
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        let host = view.read(app).popover_host.read(app);
        assert!(
            host.context_menu_focus_handle.is_focused(window),
            "opening the Add Repository menu should move focus into it"
        );
        assert_eq!(host.context_menu_selected_ix, Some(0));
    });

    simulate_key_press(cx, "down");
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app)
                .popover_host
                .read(app)
                .context_menu_selected_ix,
            Some(1),
            "Down should select Clone repository"
        );
    });

    // Space has the same activation semantics as a focused shared Button.
    simulate_key_press(cx, "space");
    cx.update(|window, app| {
        let _ = window.draw(app);
        assert_eq!(
            view.read(app)
                .popover_host
                .read(app)
                .popover_kind_for_tests(),
            Some(PopoverKind::CloneRepo),
            "Space should open the selected Clone repository prompt"
        );
    });
}
