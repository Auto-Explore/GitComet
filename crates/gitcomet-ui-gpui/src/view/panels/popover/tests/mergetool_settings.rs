use super::*;
use gitcomet_core::merge::OverviewMode;

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

/// Fake just enough resolver state for the overview row to appear: bands make
/// the column exist, and a non-empty base makes the pairwise modes meaningful.
fn seed_overview_state(view: &gpui::Entity<GitCometView>, cx: &mut gpui::VisualTestContext) {
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            pane.conflict_resolver.overview_bands =
                vec![gitcomet_core::merge::OverviewRowKind::Unchanged; 8].into();
            pane.conflict_resolver.three_way_text.base = "alpha\nbeta\n".into();
        });
    });
}

#[gpui::test]
fn overview_segments_render_within_the_menu_and_switch_the_mode(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    seed_overview_state(&view, cx);
    open_mergetool_settings_and_draw(&view, cx);

    let segment_bounds = |cx: &mut gpui::VisualTestContext, selector: &'static str| {
        cx.debug_bounds(selector)
            .unwrap_or_else(|| panic!("expected {selector} in debug bounds"))
    };
    let merge = segment_bounds(cx, "mergetool_overview_merge");
    let bc = segment_bounds(cx, "mergetool_overview_bc");
    let view_segment = segment_bounds(cx, "mergetool_view_three_way");

    // All four segments sit on one row, in order, and the row still fits the
    // menu: the widest option must not push past the menu's maximum width
    // measured from where the rows start.
    assert!(merge.size.width > gpui::px(0.0));
    assert!(bc.origin.x > merge.origin.x);
    assert!(bc.origin.y == merge.origin.y);
    assert!(bc.origin.x + bc.size.width <= view_segment.origin.x + gpui::px(420.0));

    click_debug_selector(cx, "mergetool_overview_ac");
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        assert_eq!(
            main_pane.read(app).conflict_resolver.overview_mode,
            OverviewMode::BaseVsRemote,
        );
    });
}

/// Without a conflict loaded there is no overview column, so its row is absent
/// — the pairwise modes would have nothing to compare.
#[gpui::test]
fn overview_segments_are_absent_without_an_overview_column(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    open_mergetool_settings_and_draw(&view, cx);

    assert!(cx.debug_bounds("mergetool_overview_merge").is_none());
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        assert_eq!(
            main_pane.read(app).conflict_resolver.overview_mode,
            OverviewMode::Merge,
        );
    });
}
