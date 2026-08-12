use super::*;

fn open_mergetool_settings_and_draw(
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
) {
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::MergetoolSettingsMenu,
                    gpui::point(gpui::px(320.0), gpui::px(72.0)),
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

fn click_debug_selector(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    let center = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("expected {selector} in debug bounds"))
        .center();
    cx.simulate_mouse_move(center, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(center, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_up(center, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.run_until_parked();
}

/// The view mode lives in the settings menu as a segmented control, so its
/// segments have to render and act like buttons — the menu's own keyboard
/// navigation skips them.
#[gpui::test]
fn view_mode_segments_render_and_switch_the_resolver_view(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    open_mergetool_settings_and_draw(&view, cx);
    cx.debug_bounds("mergetool_view_two_way")
        .expect("expected the 2-way segment");

    click_debug_selector(cx, "mergetool_view_three_way");
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        assert_eq!(
            main_pane.read(app).conflict_resolver.view_mode,
            ConflictResolverViewMode::ThreeWay,
        );
    });

    // The menu stays open across a segment click, the way the checkable view
    // options do, so the other segment is still clickable.
    cx.update(|_window, app| {
        assert!(view.read(app).popover_host.read(app).is_open());
    });
    click_debug_selector(cx, "mergetool_view_two_way");
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        assert_eq!(
            main_pane.read(app).conflict_resolver.view_mode,
            ConflictResolverViewMode::TwoWayDiff,
        );
    });
}

/// The minimap always shows the merge itself: kdiff3's pairwise overview modes
/// are not offered, so the menu carries no row for it even with a live column.
#[gpui::test]
fn the_minimap_has_no_row_in_the_settings_menu(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    // Seed enough resolver state that a mode row would have been shown.
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            pane.conflict_resolver.minimap_bands =
                vec![gitcomet_core::merge::MinimapRowKind::Unchanged; 8].into();
            pane.conflict_resolver.three_way_text.base = "alpha\nbeta\n".into();
        });
    });
    open_mergetool_settings_and_draw(&view, cx);

    for selector in [
        "mergetool_overview_merge",
        "mergetool_overview_ab",
        "mergetool_overview_ac",
        "mergetool_overview_bc",
    ] {
        assert!(
            cx.debug_bounds(selector).is_none(),
            "{selector} should be gone with the overview modes",
        );
    }
    // The view row is still there, so this is not just an unopened menu.
    assert!(cx.debug_bounds("mergetool_view_three_way").is_some());
}
