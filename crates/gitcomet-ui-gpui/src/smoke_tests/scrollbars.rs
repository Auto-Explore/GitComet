use super::*;

#[gpui::test]
fn uniform_list_scrollbar_allows_dragging_thumb_to_scroll(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, _cx| PanelLayoutTestView::new());
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let bounds = cx
        .debug_bounds("diff_scrollbar_test")
        .expect("expected diff_scrollbar_test in debug bounds");

    let start = gpui::point(bounds.right() - px(2.0), bounds.top() + px(6.0));
    cx.simulate_mouse_move(start, None, Modifiers::default());
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        gpui::point(start.x, start.y + px(5.0)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.simulate_mouse_move(
        gpui::point(start.x, start.y + px(60.0)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        gpui::point(start.x, start.y + px(60.0)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.run_until_parked();

    cx.update(|window, app| {
        let _ = window.draw(app);
        let offset_y = view.read(app).handle.0.borrow().base_handle.offset().y;
        assert!(
            offset_y < px(0.0),
            "expected uniform-list scrollbar drag to scroll (offset should become negative)"
        );
    });
}

#[gpui::test]
fn picker_prompt_scrollbar_thumb_visible_when_overflowing(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(PickerPromptScrollbarTestView::new);
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let bounds = cx
        .debug_bounds("picker_prompt_scrollbar")
        .expect("expected picker_prompt_scrollbar in debug bounds");
    assert!(bounds.size.height > px(50.0));

    cx.update(|_window, app| {
        let handle = &view.read(app).scroll_handle;
        assert!(
            components::Scrollbar::thumb_visible_for_test(handle, px(120.0)),
            "expected picker prompt scrollbar thumb to be visible when overflowing"
        );
    });
}

#[gpui::test]
fn picker_prompt_scrollbar_drag_scrolls_list(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(PickerPromptScrollbarTestView::new);
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let bounds = cx
        .debug_bounds("picker_prompt_scrollbar")
        .expect("expected picker_prompt_scrollbar in debug bounds");
    let start = gpui::point(bounds.right() - px(2.0), bounds.top() + px(6.0));

    cx.simulate_mouse_move(start, None, Modifiers::default());
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        gpui::point(start.x, start.y + px(5.0)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.simulate_mouse_move(
        gpui::point(start.x, start.y + px(50.0)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        gpui::point(start.x, start.y + px(50.0)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.run_until_parked();

    cx.update(|window, app| {
        let _ = window.draw(app);
        let offset_y = view.read(app).scroll_handle.offset().y;
        assert!(
            offset_y < px(0.0),
            "expected picker prompt scrollbar drag to scroll (offset should become negative)"
        );
    });
}

#[gpui::test]
fn scrollbar_thumb_visible_when_overflowing(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, _cx| ScrollbarTestView::new(50));
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.update(|_window, app| {
        let handle = &view.read(app).handle;
        assert!(
            components::Scrollbar::thumb_visible_for_test(handle, px(120.0)),
            "expected scrollbar thumb to be visible when overflowing"
        );
    });
}

#[gpui::test]
fn scrollbar_thumb_hidden_when_not_overflowing(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, _cx| ScrollbarTestView::new(2));
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.update(|_window, app| {
        let handle = &view.read(app).handle;
        assert!(
            !components::Scrollbar::thumb_visible_for_test(handle, px(120.0)),
            "expected scrollbar thumb to be hidden when not overflowing"
        );
    });
}

#[gpui::test]
fn scrollbar_allows_dragging_thumb_to_scroll(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, _cx| ScrollbarTestView::new(50));
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let bounds = cx
        .debug_bounds("test_scrollbar")
        .expect("expected test_scrollbar in debug bounds");

    let start = gpui::point(bounds.right() - px(2.0), bounds.top() + px(6.0));
    cx.simulate_mouse_move(start, None, Modifiers::default());
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());

    // First move crosses the drag threshold and starts the drag.
    cx.simulate_mouse_move(
        gpui::point(start.x, start.y + px(5.0)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    // Second move should scroll.
    cx.simulate_mouse_move(
        gpui::point(start.x, start.y + px(60.0)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        gpui::point(start.x, start.y + px(60.0)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.run_until_parked();

    cx.update(|window, app| {
        let _ = window.draw(app);
        let offset_y = view.read(app).handle.offset().y;
        assert!(
            offset_y < px(0.0),
            "expected scrollbar drag to scroll (offset should become negative)"
        );
    });
}

#[gpui::test]
fn scrollbar_drag_does_not_notify_parent_view_for_each_mouse_move(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, _cx| ScrollbarTestView::new(200));
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let notify_count = Arc::new(AtomicUsize::new(0));
    let _notify_sub = cx.update(|_window, app| {
        let notify_count = Arc::clone(&notify_count);
        view.update(app, |_this, cx| {
            cx.observe_self(move |_this, _cx| {
                notify_count.fetch_add(1, Ordering::Relaxed);
            })
        })
    });
    notify_count.store(0, Ordering::Relaxed);

    let bounds = cx
        .debug_bounds("test_scrollbar")
        .expect("expected test_scrollbar in debug bounds");

    let start = gpui::point(bounds.right() - px(2.0), bounds.top() + px(6.0));
    cx.simulate_mouse_move(start, None, Modifiers::default());
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());

    for delta in [px(5.0), px(30.0), px(60.0), px(90.0)] {
        cx.simulate_mouse_move(
            gpui::point(start.x, start.y + delta),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
    }
    cx.simulate_mouse_up(
        gpui::point(start.x, start.y + px(90.0)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.run_until_parked();

    let notifies = notify_count.load(Ordering::Relaxed);
    assert!(
        notifies <= 1,
        "expected scrollbar drag to avoid repeated parent-view notifications, got {notifies}"
    );
}

#[gpui::test]
fn scrollbar_gutter_margin_clicks_still_scroll(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, _cx| ScrollbarTestView::new(50));
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let bounds = cx
        .debug_bounds("test_scrollbar")
        .expect("expected test_scrollbar in debug bounds");

    let click = gpui::point(bounds.right() - px(2.0), bounds.bottom() - px(2.0));
    cx.simulate_mouse_move(click, None, Modifiers::default());
    cx.simulate_mouse_down(click, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(click, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();

    cx.update(|window, app| {
        let _ = window.draw(app);
        let offset_y = view.read(app).handle.offset().y;
        assert!(
            offset_y < px(0.0),
            "expected clicks inside the scrollbar gutter margin to scroll instead of falling through"
        );
    });
}

#[gpui::test]
fn scrollbar_track_uses_own_bounds_when_larger_than_surface(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, _cx| ScrollbarMismatchedBoundsView::new(100));
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let bounds = cx
        .debug_bounds("outer_scrollbar")
        .expect("expected outer_scrollbar in debug bounds");

    // Scrollbar track uses a 4px margin at top/bottom.
    let click = gpui::point(bounds.right() - px(2.0), bounds.bottom() - px(6.0));
    cx.simulate_mouse_move(click, None, Modifiers::default());
    cx.simulate_mouse_down(click, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(click, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();

    cx.update(|window, app| {
        let _ = window.draw(app);
        let offset_y = view.read(app).handle.offset().y;
        assert!(
            offset_y != px(0.0),
            "expected track click near bottom to scroll even when scrollbar is taller than the scroll surface"
        );
    });
}

struct PickerPromptScrollbarTestView {
    theme: AppTheme,
    input: gpui::Entity<components::TextInput>,
    scroll_handle: ScrollHandle,
}

impl PickerPromptScrollbarTestView {
    fn new(window: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> Self {
        let input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Filter commits".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        Self {
            theme: AppTheme::gitcomet_dark(),
            input,
            scroll_handle: ScrollHandle::new(),
        }
    }
}

impl gpui::Render for PickerPromptScrollbarTestView {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let items: std::rc::Rc<[components::PickerPromptItem]> = (0..50)
            .map(|ix| {
                components::PickerPromptItem::plain(format!(
                    "Commit {ix:02}  Synthetic history entry"
                ))
            })
            .collect::<Vec<_>>()
            .into();
        let layout = std::rc::Rc::new(components::picker_prompt_layout(&items, ""));

        div()
            .size_full()
            .bg(self.theme.colors.surface.canvas)
            .child(
                div().w(px(360.0)).child(
                    components::PickerPrompt::new(self.input.clone(), self.scroll_handle.clone())
                        .prebuilt_items(items, layout)
                        .max_height(px(120.0))
                        .render(
                            self.theme,
                            ui_scale::DEFAULT_UI_SCALE_PERCENT,
                            cx,
                            |_this, _ix, _event, _window, _cx| {},
                        ),
                ),
            )
    }
}

struct ScrollbarTestView {
    theme: AppTheme,
    handle: ScrollHandle,
    rows: usize,
}

impl ScrollbarTestView {
    fn new(rows: usize) -> Self {
        Self {
            theme: AppTheme::gitcomet_dark(),
            handle: ScrollHandle::new(),
            rows,
        }
    }
}

impl gpui::Render for ScrollbarTestView {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        _cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let theme = self.theme;
        let rows = (0..self.rows)
            .map(|ix| {
                div()
                    .id(ix)
                    .h(px(20.0))
                    .px_2()
                    .child(format!("Row {ix}"))
                    .into_any_element()
            })
            .collect::<Vec<_>>();

        div().size_full().bg(theme.colors.surface.canvas).child(
            div()
                .id("scroll_container")
                .relative()
                .w(px(200.0))
                .h(px(120.0))
                .overflow_y_scroll()
                .track_scroll(&self.handle)
                .child(div().flex().flex_col().children(rows))
                .child(
                    components::Scrollbar::new("test_scrollbar", self.handle.clone())
                        .debug_selector("test_scrollbar")
                        .render(theme),
                ),
        )
    }
}

struct ScrollbarMismatchedBoundsView {
    theme: AppTheme,
    handle: ScrollHandle,
    rows: usize,
}

impl ScrollbarMismatchedBoundsView {
    fn new(rows: usize) -> Self {
        Self {
            theme: AppTheme::gitcomet_dark(),
            handle: ScrollHandle::new(),
            rows,
        }
    }
}

impl gpui::Render for ScrollbarMismatchedBoundsView {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        _cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let theme = self.theme;
        let rows = (0..self.rows)
            .map(|ix| {
                div()
                    .id(ix)
                    .h(px(20.0))
                    .px_2()
                    .child(format!("Row {ix}"))
                    .into_any_element()
            })
            .collect::<Vec<_>>();

        // Render the scrollbar in a *larger* container than the scroll surface to ensure the
        // scrollbar uses its own bounds (not the scroll handle's bounds) for hit-testing/metrics.
        div().size_full().bg(theme.colors.surface.canvas).child(
            div()
                .id("outer_scrollbar_container")
                .relative()
                .w(px(200.0))
                .h(px(200.0))
                .child(
                    div()
                        .id("inner_scroll_surface")
                        .relative()
                        .w_full()
                        .h(px(120.0))
                        .overflow_y_scroll()
                        .track_scroll(&self.handle)
                        .child(div().flex().flex_col().children(rows)),
                )
                .child(
                    components::Scrollbar::new("outer_scrollbar", self.handle.clone())
                        .debug_selector("outer_scrollbar")
                        .render(theme),
                ),
        )
    }
}
