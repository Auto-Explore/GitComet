use super::*;

#[gpui::test]
fn long_tooltips_wrap_to_the_visible_window_width(cx: &mut gpui::TestAppContext) {
    crate::test_support::use_real_text_backend(cx);
    let (host, cx) = cx.add_window_view(|_window, _cx| TooltipHost::new(AppTheme::gitcomet_dark()));
    let mut short_size = None;
    for (width, anchor_x, right_margin) in [
        (320.0, 0.0, 0.0),
        (320.0, 300.0, 8.0),
        (640.0, 0.0, 0.0),
        (640.0, 620.0, 8.0),
    ] {
        host.update(cx, |host, cx| {
            host.on_mouse_moved(point(px(anchor_x), px(0.0)), cx);
            host.set_tooltip_text_if_changed(
                Some("git fetch failed: the remote repository could not be read; check the repository address and access permissions. ".repeat(4).into()),
                cx,
            );
        });
        cx.simulate_resize(gpui::size(px(width), px(460.0)));
        crate::test_support::redraw(cx);
        let bubble = cx.debug_bounds("tooltip_content").expect("visible tooltip");
        let viewport = cx.update(|window, _| window.viewport_size());
        assert_eq!(viewport.width, px(width));
        assert!(bubble.left() >= px(8.0));
        assert!(
            bubble.right() <= px(width - right_margin),
            "tooltip {bubble:?} does not fit viewport {viewport:?}"
        );
        assert!(bubble.top() >= px(8.0));
        assert!(bubble.bottom() <= px(452.0));
        assert!(bubble.size.height > px(32.0));

        host.update(cx, |host, cx| {
            host.set_tooltip_text_if_changed(Some("Fetch failed".into()), cx);
        });
        crate::test_support::redraw(cx);
        let short = cx.debug_bounds("tooltip_content").unwrap();
        assert!(short.size.height <= px(32.0));
        if let Some(previous) = short_size {
            assert_eq!(short.size, previous);
        }
        short_size = Some(short.size);
    }
}
