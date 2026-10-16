use super::*;

#[test]
fn builds_pure_components_without_panics() {
    for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
        assert_no_panic("components::pill", || {
            let _ = components::pill(theme, "Label", theme.colors.accent.foreground);
        });

        assert_no_panic("components::empty_state", || {
            let _ = components::empty_state(theme, "Title", "Message");
        });

        assert_no_panic("components::empty_state_message", || {
            let _ = components::empty_state_message(theme, "Message");
        });

        assert_no_panic("components::panel", || {
            let _ = components::panel(theme, "Panel", None, div().child("body"));
        });

        assert_no_panic("components::diff_stat", || {
            let _ = components::diff_stat(theme, ui_scale::DEFAULT_UI_SCALE_PERCENT, 12, 4);
        });

        assert_no_panic("components::toast", || {
            let _ = components::toast(
                theme,
                ui_scale::DEFAULT_UI_SCALE_PERCENT,
                components::ToastKind::Success,
                "Hello",
            );
        });

        assert_no_panic("components::Button render variants", || {
            let _ = components::Button::new("z1", "Filled")
                .style(components::ButtonStyle::Filled)
                .render(theme, ui_scale::DEFAULT_UI_SCALE_PERCENT);
            let _ = components::Button::new("z2", "Outlined")
                .style(components::ButtonStyle::Outlined)
                .render(theme, ui_scale::DEFAULT_UI_SCALE_PERCENT);
            let _ = components::Button::new("z3", "Subtle")
                .style(components::ButtonStyle::Subtle)
                .render(theme, ui_scale::DEFAULT_UI_SCALE_PERCENT);
            let _ = components::Button::new("z4", "Disabled")
                .style(components::ButtonStyle::Outlined)
                .disabled(true)
                .render(theme, ui_scale::DEFAULT_UI_SCALE_PERCENT);
            let _ = components::Button::new("z5", "Create")
                .style(components::ButtonStyle::Filled)
                .separated_end_slot(div().text_size(theme.ui_text(12.0)).child("Enter"))
                .render(theme, ui_scale::DEFAULT_UI_SCALE_PERCENT);
        });

        assert_no_panic("components::SplitButton", || {
            let left = components::Button::new("s1", "Left")
                .style(components::ButtonStyle::Outlined)
                .render(theme, ui_scale::DEFAULT_UI_SCALE_PERCENT);
            let right = components::Button::new("s2", "Right")
                .style(components::ButtonStyle::Outlined)
                .render(theme, ui_scale::DEFAULT_UI_SCALE_PERCENT);
            let _ = components::SplitButton::new(left, right)
                .style(components::SplitButtonStyle::Borderless)
                .render(theme, ui_scale::DEFAULT_UI_SCALE_PERCENT);
        });

        assert_no_panic("components::Tab + TabBar", || {
            let tab = components::Tab::new(("t", 1u64))
                .selected(true)
                .child(div().child("Repo"))
                .render(theme);
            let _ = components::TabBar::new("tb").tab(tab).render();
        });

        assert_no_panic("view::window_frame", || {
            let content = div().child("content").into_any_element();
            let _ = view::window_frame(
                theme,
                Decorations::Server,
                content,
                None,
                ui_scale::DEFAULT_UI_SCALE_PERCENT,
            );
            let _ = view::window_frame(
                theme,
                Decorations::Client {
                    tiling: Tiling::default(),
                },
                div().child("content").into_any_element(),
                None,
                ui_scale::DEFAULT_UI_SCALE_PERCENT,
            );
        });

        assert_no_panic("window-frame uses rounding/border", || {
            let _ = div().rounded(px(theme.radii.panel)).border_1().child("x");
        });
    }
}

#[gpui::test]
fn smoke_view_renders_without_panicking(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        cx.open_window(Default::default(), |window, cx| {
            cx.new(|cx| SmokeView::new(window, cx))
        })
        .unwrap();
    });
}

#[gpui::test]
fn tab_selection_border_keeps_content_inset_stable(cx: &mut gpui::TestAppContext) {
    let (_view, cx) = cx.add_window_view(SmokeView::new);
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let selected_tab = cx
        .debug_bounds("smoke_selected_tab")
        .expect("expected the selected smoke tab");
    let selected_content = cx
        .debug_bounds("smoke_selected_tab_content")
        .expect("expected the selected smoke tab content");
    let idle_tab = cx
        .debug_bounds("smoke_idle_tab")
        .expect("expected the idle smoke tab");
    let idle_content = cx
        .debug_bounds("smoke_idle_tab_content")
        .expect("expected the idle smoke tab content");

    assert_eq!(
        selected_content.left() - selected_tab.left(),
        idle_content.left() - idle_tab.left(),
        "selected and idle tabs must reserve the same leading border inset",
    );
    assert_eq!(
        selected_content.top() - selected_tab.top(),
        idle_content.top() - idle_tab.top(),
        "selected and idle tabs must reserve the same top border inset",
    );
}

#[gpui::test]
fn gitcomet_view_renders_without_panicking(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        cx.open_window(Default::default(), |window, cx| {
            cx.new(|cx| crate::view::GitCometView::new(store, events, None, window, cx))
        })
        .unwrap();
    });
}

#[gpui::test]
fn panel_allows_flex_body_to_have_height(cx: &mut gpui::TestAppContext) {
    let (_view, cx) = cx.add_window_view(|_window, _cx| PanelLayoutTestView::new());
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let bounds = cx
        .debug_bounds("diff_body")
        .expect("expected diff_body to be painted");
    assert!(bounds.size.height > px(50.0));
}

fn assert_no_panic(label: &str, f: impl FnOnce()) {
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_err() {
        panic!("component build panicked: {label}");
    }
}
