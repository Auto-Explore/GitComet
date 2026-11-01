use super::extension_view_panels::{click, open_view, shows};
use super::*;

#[cfg(target_os = "linux")]
mod capture;

#[gpui::test]
fn all_sidebar_tabs_share_one_track_and_keep_the_active_label(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_view(cx);
    cx.update(|_, app| {
        let view = view.read(app);
        view.store
            .replace_snapshot_for_test(view.ui_model.read(app).state.clone());
    });
    click(cx, "repository_view_0");
    click(cx, "sidebar_tab_branches");
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            test_support::set_sidebar_width_for_test(view, px(200.0), cx)
        })
    });
    test_support::redraw(cx);
    let track = cx.debug_bounds("sidebar_view_toggle").unwrap();
    let search = cx.debug_bounds("sidebar_search_toggle").unwrap();
    for selector in [
        "sidebar_tab_branches",
        "sidebar_tab_files",
        "sidebar_tab_extension_0",
        "sidebar_tab_extension_1",
    ] {
        let tab = cx.debug_bounds(selector).unwrap();
        assert_eq!(
            tab.top(),
            cx.debug_bounds("sidebar_tab_branches").unwrap().top()
        );
        assert!(tab.left() >= track.left());
        assert!(tab.right() <= search.left());
        assert!(tab.bottom() <= track.bottom());
    }
    assert!(shows(cx, "sidebar_tab_branches_label"));
    assert!(!shows(cx, "sidebar_tab_extension_1_label"));
    assert!(shows(cx, "sidebar_tab_extension_1_icon"));
    assert!(cx.update(|_, app| view.read(app).sidebar_width) > px(200.0));

    click(cx, "sidebar_tab_extension_1");
    assert!(shows(cx, "panels_extra_history"));
    assert!(shows(cx, "sidebar_tab_extension_1_label"));
    assert!(!shows(cx, "sidebar_search_toggle"));
    let expanded = cx.update(|_, app| view.read(app).sidebar_width);
    let label = cx.debug_bounds("sidebar_tab_extension_1_label").unwrap();
    let tab = cx.debug_bounds("sidebar_tab_extension_1").unwrap();
    assert!(label.right() <= tab.right());
    assert!(tab.right() <= cx.debug_bounds("sidebar_header").unwrap().right());

    click(cx, "sidebar_tab_files");
    pump_until(cx, "Files sidebar selection", |cx| {
        cx.update(|_, app| {
            view.read(app).store.snapshot().sidebar_mode
                == gitcomet_state::model::SidebarMode::Files
        })
    });
    sync_view_snapshot(cx, &view);
    assert!(shows(cx, "sidebar_search_toggle"));
    assert!(shows(cx, "sidebar_explorer_settings"));
    assert!(shows(cx, "sidebar_locate_open_file"));
    assert!(cx.update(|_, app| view.read(app).sidebar_width) >= expanded);
    click(cx, "sidebar_tab_extension_0");
    assert!(shows(cx, "panels_files"));
    assert!(!shows(cx, "sidebar_locate_open_file"));
}

#[gpui::test]
fn sidebar_heading_uses_content_header_surface_and_preserves_body_surface(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = open_view(cx);
    for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.theme = theme;
                view.sidebar_pane.update(cx, |pane, cx| {
                    pane.theme = theme;
                    cx.notify();
                });
                cx.notify();
            })
        });
        test_support::redraw(cx);
        let header: gpui::Background = crate::theme::content_header_bg(theme).into();
        assert!(
            crate::kit::test_support::painted_control_quads(cx, "sidebar_header")
                .iter()
                .any(|(fill, _)| *fill == header)
        );
        let body: gpui::Background = theme.colors.surface.chrome.into();
        assert!(
            crate::kit::test_support::painted_control_quads(cx, "sidebar_pane")
                .iter()
                .any(|(fill, _)| *fill == body)
        );
    }
}

#[gpui::test]
fn header_minimum_follows_scale_history_and_sidebar_expansion(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_view(cx);
    click(cx, "repository_view_0");
    click(cx, "sidebar_tab_extension_1");
    for percent in [100, 150] {
        cx.update(|window, app| {
            view.update(app, |view, cx| {
                view.apply_ui_scale_percent(percent, window, cx);
                test_support::set_sidebar_width_for_test(view, px(1.0), cx);
            })
        });
        test_support::redraw(cx);
        let (width, minimum) = cx.update(|_, app| {
            let view = view.read(app);
            (view.sidebar_width, view.sidebar_min_width())
        });
        assert_eq!(width, minimum);
        assert!(
            cx.debug_bounds("sidebar_tab_extension_1").unwrap().right()
                <= cx.debug_bounds("sidebar_header").unwrap().right()
        );
    }
    cx.update(|window, app| {
        view.update(app, |view, cx| {
            view.open_file_history(RepoId(1), "src/main.rs".into(), window, cx)
        })
    });
    cx.run_until_parked();
    test_support::redraw(cx);
    assert!(shows(cx, "sidebar_tab_file_history_label"));
    let history = cx.debug_bounds("sidebar_tab_file_history").unwrap();
    assert!(history.right() <= cx.debug_bounds("sidebar_header").unwrap().right());
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.set_sidebar_collapsed(true, cx);
            view.set_sidebar_collapsed(false, cx);
        })
    });
    cx.run_until_parked();
    test_support::redraw(cx);
    assert!(shows(cx, "sidebar_tab_file_history_label"));
    assert!(cx.update(|_, app| view.read(app).sidebar_width >= view.read(app).sidebar_min_width()));
    cx.simulate_resize(gpui::size(px(500.0), px(450.0)));
    test_support::redraw(cx);
    let requested = cx.update(|window, app| {
        let view = view.read(app);
        let requested = window.bounds().size;
        assert_eq!(requested.height, px(450.0));
        assert!(
            requested.width
                >= view.sidebar_min_width() + view.main_min_width() + view.details_min_width()
        );
        requested
    });
    cx.simulate_resize(requested);
    test_support::redraw(cx);
    assert_eq!(cx.update(|window, _| window.viewport_size()), requested);
}

#[test]
fn pane_drag_uses_the_scaled_header_minimum_after_window_resize() {
    let state = PaneResizeState::new(
        PaneResizeHandle::Sidebar,
        px(0.0),
        px(450.0),
        px(420.0),
        px(1600.0),
        false,
        false,
    )
    .with_minimums(px(390.0), px(420.0), px(51.0));
    assert_eq!(
        next_pane_resize_drag_width(&state, px(-900.0), px(1600.0), false, false),
        px(390.0)
    );
    assert_eq!(
        next_pane_resize_drag_width(&state, px(-900.0), px(1200.0), false, false),
        px(390.0)
    );
    assert_eq!(
        state.drag_width_bounds(px(1200.0), false, true),
        (px(390.0), px(729.0))
    );
}
