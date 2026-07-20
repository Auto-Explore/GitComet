use super::*;

#[test]
fn popover_width_spec_scales_with_zoom() {
    let spec = popover_width_spec(&PopoverKind::RepoPicker).expect("repo picker width");
    let default_scale = ui_scale::UiScale::from_percent(100);
    let zoomed_scale = ui_scale::UiScale::from_percent(200);

    assert_eq!(spec.preferred_px(default_scale), px(420.0));
    assert_eq!(spec.preferred_px(zoomed_scale), px(840.0));
    assert_eq!(spec.max_px(zoomed_scale), px(1640.0));
}

#[test]
fn choose_popover_anchor_corner_prefers_side_with_more_space() {
    assert_eq!(
        choose_popover_anchor_corner(Anchor::TopRight, px(260.0), px(640.0), px(420.0),),
        Anchor::TopLeft,
    );
    assert_eq!(
        choose_popover_anchor_corner(Anchor::BottomLeft, px(500.0), px(260.0), px(420.0),),
        Anchor::BottomRight,
    );
}

#[gpui::test]
fn reword_dialog_with_long_squash_message_stays_within_viewport(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let description = (0..120)
        .map(|ix| format!("Squashed commit message {ix}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let original_message = format!("Combined subject\n\n{description}");

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_centered(
                    PopoverKind::RebaseReword {
                        ix: 0,
                        original_action: InteractiveRebaseAction::Pick,
                        original_message,
                    },
                    window,
                    cx,
                );
            });
        });
    });
    crate::view::test_support::redraw(cx);

    let popover_bounds = cx
        .debug_bounds("app_popover")
        .expect("expected reword dialog to render");
    let mut viewport_height = px(0.0);
    cx.update(|window, _app| {
        viewport_height = window.window_bounds().get_bounds().size.height;
    });

    assert!(
        popover_bounds.bottom() <= viewport_height,
        "reword dialog bottom {:?} exceeded viewport height {:?}",
        popover_bounds.bottom(),
        viewport_height,
    );
}
