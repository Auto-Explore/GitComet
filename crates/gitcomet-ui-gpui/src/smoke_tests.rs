use crate::test_support::{lock_clipboard_test, lock_visual_test};
use crate::view::components;
use crate::{theme::AppTheme, ui_scale, view};
use gitcomet_core::domain::*;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{GitBackend, GitRepository, PullMode, Result};
use gitcomet_state::model::Loadable;
use gitcomet_state::model::RepoId;
use gitcomet_state::msg::Msg;
use gitcomet_state::store::AppStore;
use gpui::prelude::*;
use gpui::{
    ClipboardItem, Decorations, KeyBinding, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent,
    Pixels, ScrollDelta, ScrollHandle, ScrollWheelEvent, SharedString, Tiling, div, px,
};
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

fn assert_no_panic(label: &str, f: impl FnOnce()) {
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_err() {
        panic!("component build panicked: {label}");
    }
}

fn abs_scroll_y(raw: Pixels) -> Pixels {
    if raw < px(0.0) { -raw } else { raw }
}

fn open_text_input_context_menu(cx: &mut gpui::VisualTestContext, position: gpui::Point<Pixels>) {
    cx.simulate_mouse_move(position, None, Modifiers::default());
    cx.simulate_event(MouseDownEvent {
        position,
        modifiers: Modifiers::default(),
        button: MouseButton::Right,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position,
        modifiers: Modifiers::default(),
        button: MouseButton::Right,
        click_count: 1,
    });
}

#[test]
fn builds_pure_components_without_panics() {
    for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
        assert_no_panic("components::pill", || {
            let _ = components::pill(theme, "Label", theme.colors.accent);
        });

        assert_no_panic("components::empty_state", || {
            let _ = components::empty_state(theme, "Title", "Message");
        });

        assert_no_panic("components::panel", || {
            let _ = components::panel(theme, "Panel", None, div().child("body"));
        });

        assert_no_panic("components::diff_stat", || {
            let _ = components::diff_stat(theme, 12, 4);
        });

        assert_no_panic("components::toast", || {
            let _ = components::toast(theme, components::ToastKind::Success, "Hello");
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
                .separated_end_slot(div().text_xs().child("Enter"))
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
                .style(components::SplitButtonStyle::Outlined)
                .render(theme, ui_scale::DEFAULT_UI_SCALE_PERCENT);
        });

        assert_no_panic("components::Tab + TabBar", || {
            let tab = components::Tab::new(("t", 1u64))
                .selected(true)
                .child(div().child("Repo"))
                .render(theme, ui_scale::DEFAULT_UI_SCALE_PERCENT);
            let _ = components::TabBar::new("tb")
                .tab(tab)
                .render(theme, ui_scale::DEFAULT_UI_SCALE_PERCENT);
        });

        assert_no_panic("view::window_frame", || {
            let content = div().child("content").into_any_element();
            let _ = view::window_frame(
                theme,
                Decorations::Server,
                content,
                ui_scale::DEFAULT_UI_SCALE_PERCENT,
            );
            let _ = view::window_frame(
                theme,
                Decorations::Client {
                    tiling: Tiling::default(),
                },
                div().child("content").into_any_element(),
                ui_scale::DEFAULT_UI_SCALE_PERCENT,
            );
        });

        assert_no_panic("window-frame uses shadow/rounding", || {
            let _ = div()
                .rounded(px(theme.radii.panel))
                .shadow_lg()
                .border_1()
                .child("x");
        });
    }
}

struct SmokeView {
    theme: AppTheme,
    input: gpui::Entity<components::TextInput>,
}

impl SmokeView {
    fn new(window: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> Self {
        let input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Enter".into(),
                    multiline: false,
                    read_only: false,
                    chromeless: false,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });

        Self {
            theme: AppTheme::gitcomet_dark(),
            input,
        }
    }
}

impl gpui::Render for SmokeView {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        _cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let theme = self.theme;
        let tabs = components::TabBar::new("smoke_tabs")
            .tab(
                components::Tab::new(("t", 0u64))
                    .selected(true)
                    .child(div().child("One"))
                    .render(theme, ui_scale::DEFAULT_UI_SCALE_PERCENT),
            )
            .tab(
                components::Tab::new(("t", 1u64))
                    .selected(false)
                    .child(div().child("Two"))
                    .render(theme, ui_scale::DEFAULT_UI_SCALE_PERCENT),
            )
            .render(theme, ui_scale::DEFAULT_UI_SCALE_PERCENT);

        let content = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(components::panel(theme, "Tabs", None, tabs))
            .child(components::panel(
                theme,
                "Input",
                None,
                div()
                    .id("smoke_input")
                    .debug_selector(|| "smoke_input".to_string())
                    .child(self.input.clone()),
            ))
            .child(components::panel(
                theme,
                "Buttons",
                None,
                div()
                    .flex()
                    .gap_2()
                    .child(
                        components::Button::new("b1", "Primary")
                            .style(components::ButtonStyle::Filled)
                            .render(theme, ui_scale::DEFAULT_UI_SCALE_PERCENT),
                    )
                    .child(
                        components::Button::new("b2", "Secondary")
                            .style(components::ButtonStyle::Outlined)
                            .render(theme, ui_scale::DEFAULT_UI_SCALE_PERCENT),
                    ),
            ))
            .into_any_element();

        view::window_frame(
            theme,
            window.window_decorations(),
            content,
            ui_scale::DEFAULT_UI_SCALE_PERCENT,
        )
    }
}

struct TextInputHostView {
    theme: AppTheme,
    input: gpui::Entity<components::TextInput>,
}

struct TextInputCursorScrollView {
    theme: AppTheme,
    input: gpui::Entity<components::TextInput>,
    scroll_handle: ScrollHandle,
}

impl TextInputCursorScrollView {
    fn new(window: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> Self {
        let scroll_handle = ScrollHandle::new();
        let input = cx.new({
            let scroll_handle = scroll_handle.clone();
            move |cx| {
                let mut input = components::TextInput::new(
                    components::TextInputOptions {
                        placeholder: "Enter".into(),
                        multiline: true,
                        read_only: false,
                        chromeless: false,
                        soft_wrap: true,
                    },
                    window,
                    cx,
                );
                input.set_vertical_scroll_handle(Some(scroll_handle.clone()));
                input
            }
        });

        Self {
            theme: AppTheme::gitcomet_dark(),
            input,
            scroll_handle,
        }
    }
}

impl gpui::Render for TextInputCursorScrollView {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        _cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let theme = self.theme;
        let content = div()
            .flex()
            .flex_col()
            .p_2()
            .child(
                div()
                    .id("cursor_scroll_surface")
                    .relative()
                    .w(px(280.0))
                    .h(px(100.0))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll_handle)
                    .child(self.input.clone())
                    .child(
                        components::Scrollbar::new("cursor_scrollbar", self.scroll_handle.clone())
                            .render(theme),
                    ),
            )
            .into_any_element();

        view::window_frame(
            theme,
            window.window_decorations(),
            content,
            ui_scale::DEFAULT_UI_SCALE_PERCENT,
        )
    }
}

impl TextInputHostView {
    fn new(window: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> Self {
        let input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Enter".into(),
                    multiline: false,
                    read_only: false,
                    chromeless: false,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });

        Self {
            theme: AppTheme::gitcomet_dark(),
            input,
        }
    }
}

impl gpui::Render for TextInputHostView {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        _cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let content = div()
            .flex()
            .flex_col()
            .p_2()
            .child(
                div()
                    .id("smoke_input")
                    .debug_selector(|| "smoke_input".to_string())
                    .child(self.input.clone()),
            )
            .into_any_element();

        view::window_frame(
            self.theme,
            window.window_decorations(),
            content,
            ui_scale::DEFAULT_UI_SCALE_PERCENT,
        )
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
fn text_input_constructs_without_panicking(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        cx.open_window(Default::default(), |window, cx| {
            cx.new(|cx| {
                components::TextInput::new(
                    components::TextInputOptions {
                        placeholder: "Commit message".into(),
                        multiline: false,
                        read_only: false,
                        chromeless: false,
                        soft_wrap: false,
                    },
                    window,
                    cx,
                )
            })
        })
        .unwrap();
    });
}

#[gpui::test]
fn text_input_focus_after_initial_draw_accepts_typed_input(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(TextInputHostView::new);

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);
        let _ = window.draw(app);
    });

    cx.simulate_input("x");

    let text = cx.update(|window, app| {
        let _ = window.draw(app);
        view.read(app).input.read(app).text().to_string()
    });
    assert_eq!(text, "x");
}

#[gpui::test]
fn text_input_supports_basic_clipboard_and_word_shortcuts(cx: &mut gpui::TestAppContext) {
    let _clipboard_guard = lock_clipboard_test();
    let (view, cx) = cx.add_window_view(SmokeView::new);

    cx.update(|window, app| {
        app.bind_keys([
            KeyBinding::new("ctrl-a", crate::kit::SelectAll, Some("TextInput")),
            KeyBinding::new("ctrl-c", crate::kit::Copy, Some("TextInput")),
            KeyBinding::new("ctrl-x", crate::kit::Cut, Some("TextInput")),
            KeyBinding::new("ctrl-v", crate::kit::Paste, Some("TextInput")),
            KeyBinding::new("ctrl-left", crate::kit::WordLeft, Some("TextInput")),
            KeyBinding::new(
                "ctrl-backspace",
                crate::kit::DeleteWordLeft,
                Some("TextInput"),
            ),
            KeyBinding::new(
                "ctrl-delete",
                crate::kit::DeleteWordRight,
                Some("TextInput"),
            ),
            KeyBinding::new(
                "ctrl-shift-left",
                crate::kit::SelectWordLeft,
                Some("TextInput"),
            ),
        ]);

        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);

        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("hello world", cx));
        });
    });

    cx.simulate_keystrokes("ctrl-a ctrl-c");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("hello world".into())
    );

    cx.simulate_keystrokes("ctrl-x");
    let text = cx.update(|_window, app| view.read(app).input.read(app).text().to_string());
    assert_eq!(text, "");

    cx.write_to_clipboard(ClipboardItem::new_string("abc".to_string()));
    cx.simulate_keystrokes("ctrl-v");
    let text = cx.update(|_window, app| view.read(app).input.read(app).text().to_string());
    assert_eq!(text, "abc");

    cx.update(|window, app| {
        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);
        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("hello world", cx));
        });
    });
    cx.simulate_keystrokes("ctrl-shift-left ctrl-c");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("world".into())
    );

    cx.update(|window, app| {
        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);
        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("hello brave world", cx));
        });
    });

    cx.simulate_keystrokes("ctrl-backspace");
    let text = cx.update(|_window, app| view.read(app).input.read(app).text().to_string());
    assert_eq!(text, "hello brave ");

    cx.update(|window, app| {
        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);
        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("hello brave world", cx));
        });
    });

    cx.simulate_keystrokes("ctrl-left ctrl-delete");
    let text = cx.update(|_window, app| view.read(app).input.read(app).text().to_string());
    assert_eq!(text, "hello brave ");
}

#[gpui::test]
fn text_input_shift_backspace_deletes_like_backspace(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(SmokeView::new);

    cx.update(|window, app| {
        app.bind_keys([
            KeyBinding::new("backspace", crate::kit::Backspace, Some("TextInput")),
            KeyBinding::new("shift-backspace", crate::kit::Backspace, Some("TextInput")),
        ]);

        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);

        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("hello", cx));
        });
    });

    cx.simulate_keystrokes("backspace");
    let plain_backspace =
        cx.update(|_window, app| view.read(app).input.read(app).text().to_string());
    assert_eq!(plain_backspace, "hell");

    cx.update(|window, app| {
        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);

        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("hello", cx));
        });
    });

    cx.simulate_keystrokes("shift-backspace");
    let shift_backspace =
        cx.update(|_window, app| view.read(app).input.read(app).text().to_string());
    assert_eq!(shift_backspace, plain_backspace);
}

#[gpui::test]
fn multiline_text_input_cursor_navigation_keeps_scroll_in_view(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(TextInputCursorScrollView::new);

    cx.update(|window, app| {
        app.bind_keys([
            KeyBinding::new("enter", crate::kit::Enter, Some("TextInput")),
            KeyBinding::new("up", crate::kit::Up, Some("TextInput")),
            KeyBinding::new("down", crate::kit::Down, Some("TextInput")),
        ]);

        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);

        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("line".to_string(), cx));
        });

        let _ = window.draw(app);
    });

    cx.simulate_keystrokes("enter enter enter enter enter enter enter enter enter enter");
    cx.run_until_parked();
    let (after_enter, max_after_enter) = cx.update(|window, app| {
        let _ = window.draw(app);
        let v = view.read(app);
        (
            abs_scroll_y(v.scroll_handle.offset().y),
            v.scroll_handle.max_offset().y,
        )
    });
    assert!(
        after_enter > px(0.0),
        "expected Enter to move cursor down and auto-scroll to keep it visible"
    );
    assert!(
        max_after_enter <= px(0.0) || after_enter >= max_after_enter - px(1.0),
        "expected Enter at EOF to keep scroll pinned to bottom"
    );

    cx.simulate_keystrokes("up up up up up up up up up up");
    cx.run_until_parked();
    let after_up = cx.update(|window, app| {
        let _ = window.draw(app);
        abs_scroll_y(view.read(app).scroll_handle.offset().y)
    });
    assert!(
        after_up < after_enter,
        "expected Up navigation to scroll back upward with cursor"
    );

    cx.simulate_keystrokes("down down down down down down down down down down");
    cx.run_until_parked();
    let after_down = cx.update(|window, app| {
        let _ = window.draw(app);
        abs_scroll_y(view.read(app).scroll_handle.offset().y)
    });
    assert!(
        after_down > after_up,
        "expected Down navigation to scroll downward with cursor"
    );
}

#[gpui::test]
fn multiline_text_input_mousewheel_does_not_trigger_cursor_autoscroll(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = cx.add_window_view(TextInputCursorScrollView::new);

    cx.update(|window, app| {
        app.bind_keys([KeyBinding::new(
            "enter",
            crate::kit::Enter,
            Some("TextInput"),
        )]);

        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);

        let long_text = (0..40)
            .map(|ix| format!("line {ix}"))
            .collect::<Vec<_>>()
            .join("\n");
        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text(long_text, cx));
        });

        let _ = window.draw(app);
    });

    // Set a deterministic non-bottom scroll position before wheel input.
    let (before_wheel, max_offset) = cx.update(|window, app| {
        let _ = window.draw(app);
        let (scroll_handle, max_offset) = {
            let v = view.read(app);
            (
                v.scroll_handle.clone(),
                v.scroll_handle.max_offset().y.max(px(0.0)),
            )
        };
        let baseline = (max_offset * 0.5).max(px(1.0));
        scroll_handle.set_offset(gpui::point(px(0.0), -baseline.min(max_offset)));
        let _ = window.draw(app);
        (abs_scroll_y(scroll_handle.offset().y), max_offset)
    });
    assert!(
        max_offset > px(0.0),
        "expected multiline content to overflow"
    );
    assert!(
        before_wheel > px(0.0) && before_wheel < max_offset,
        "expected baseline scroll offset to be between top and bottom"
    );

    let surface_bounds = cx.update(|window, app| {
        let _ = window.draw(app);
        view.read(app).scroll_handle.bounds()
    });
    cx.simulate_event(ScrollWheelEvent {
        position: surface_bounds.center(),
        delta: ScrollDelta::Pixels(gpui::point(px(0.0), px(-120.0))),
        ..Default::default()
    });
    cx.run_until_parked();

    let after_wheel = cx.update(|window, app| {
        let _ = window.draw(app);
        let v = view.read(app);
        abs_scroll_y(v.scroll_handle.offset().y)
    });

    let wheel_delta = if after_wheel >= before_wheel {
        after_wheel - before_wheel
    } else {
        before_wheel - after_wheel
    };
    assert!(
        wheel_delta > px(0.5),
        "expected mousewheel to move scroll (before={before_wheel:?}, after={after_wheel:?})"
    );
    assert!(
        after_wheel < max_offset - px(1.0),
        "expected mousewheel not to snap back to bottom (after={after_wheel:?}, max={max_offset:?})"
    );
}

#[gpui::test]
fn text_input_right_click_context_menu_supports_copy(cx: &mut gpui::TestAppContext) {
    let _clipboard_guard = lock_clipboard_test();
    let (view, cx) = cx.add_window_view(SmokeView::new);

    cx.update(|window, app| {
        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);

        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("hello world", cx));
        });

        let _ = window.draw(app);
    });

    cx.write_to_clipboard(ClipboardItem::new_string("initial".to_string()));

    let bounds = cx
        .debug_bounds("smoke_input")
        .expect("expected smoke input bounds");
    let click = bounds.center();

    cx.simulate_mouse_move(click, None, Modifiers::default());
    cx.simulate_event(MouseDownEvent {
        position: click,
        modifiers: Modifiers::default(),
        button: MouseButton::Right,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: click,
        modifiers: Modifiers::default(),
        button: MouseButton::Right,
        click_count: 1,
    });

    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("initial".into())
    );

    let select_all_bounds = cx
        .debug_bounds("text_input_context_select_all")
        .expect("expected text-input select-all context menu row");
    let select_all_click = select_all_bounds.center();

    cx.simulate_mouse_move(select_all_click, None, Modifiers::default());
    cx.simulate_event(MouseDownEvent {
        position: select_all_click,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: select_all_click,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
    });

    // Open menu again while full selection is active, then copy from the menu.
    cx.simulate_mouse_move(click, None, Modifiers::default());
    cx.simulate_event(MouseDownEvent {
        position: click,
        modifiers: Modifiers::default(),
        button: MouseButton::Right,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: click,
        modifiers: Modifiers::default(),
        button: MouseButton::Right,
        click_count: 1,
    });

    let copy_bounds = cx
        .debug_bounds("text_input_context_copy")
        .expect("expected text-input copy context menu row");
    let copy_click = copy_bounds.center();

    cx.simulate_mouse_move(copy_click, None, Modifiers::default());
    cx.simulate_event(MouseDownEvent {
        position: copy_click,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: copy_click,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
    });

    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("hello world".into())
    );
}

#[gpui::test]
fn text_input_context_menu_does_not_resize_input_container(cx: &mut gpui::TestAppContext) {
    let (_view, cx) = cx.add_window_view(TextInputHostView::new);

    let before = cx
        .debug_bounds("smoke_input")
        .expect("expected smoke input bounds before opening context menu");
    let click = before.center();

    cx.simulate_mouse_move(click, None, Modifiers::default());
    cx.simulate_event(MouseDownEvent {
        position: click,
        modifiers: Modifiers::default(),
        button: MouseButton::Right,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: click,
        modifiers: Modifiers::default(),
        button: MouseButton::Right,
        click_count: 1,
    });

    let _ = cx
        .debug_bounds("text_input_context_select_all")
        .expect("expected text-input context menu to be open");

    let after = cx
        .debug_bounds("smoke_input")
        .expect("expected smoke input bounds after opening context menu");
    let width_delta = (f32::from(after.size.width) - f32::from(before.size.width)).abs();
    let height_delta = (f32::from(after.size.height) - f32::from(before.size.height)).abs();
    assert!(
        width_delta <= 0.1 && height_delta <= 0.1,
        "expected input bounds to stay stable when context menu opens; before=({}, {}) after=({}, {})",
        f32::from(before.size.width),
        f32::from(before.size.height),
        f32::from(after.size.width),
        f32::from(after.size.height)
    );
}

#[gpui::test]
fn text_input_context_menu_grows_wider_with_ui_zoom(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(SmokeView::new);

    cx.update(|window, app| {
        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);

        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("hello world", cx));
        });

        let _ = window.draw(app);
    });

    let bounds = cx
        .debug_bounds("smoke_input")
        .expect("expected smoke input bounds");
    let click = bounds.center();

    open_text_input_context_menu(cx, click);

    let default_row_width: f32 = cx
        .debug_bounds("text_input_context_select_all")
        .expect("expected text-input context menu row before zooming")
        .size
        .width
        .into();

    cx.update(|window, app| {
        view.update(app, |_this, cx| {
            crate::ui_scale::set_current(cx, 200);
        });
        crate::ui_scale::apply_to_window(window, 200);
        let _ = window.draw(app);
    });
    cx.run_until_parked();

    let zoomed_row_width: f32 = cx
        .debug_bounds("text_input_context_select_all")
        .expect("expected text-input context menu row after zooming")
        .size
        .width
        .into();

    assert!(
        zoomed_row_width > default_row_width * 1.6,
        "expected the text-input context menu to grow substantially with zoom (default={default_row_width}, zoomed={zoomed_row_width})"
    );
}

#[gpui::test]
fn text_input_supports_ctrl_z_undo(cx: &mut gpui::TestAppContext) {
    let _clipboard_guard = lock_clipboard_test();
    let (view, cx) = cx.add_window_view(SmokeView::new);

    cx.update(|window, app| {
        app.bind_keys([
            KeyBinding::new("ctrl-v", crate::kit::Paste, Some("TextInput")),
            KeyBinding::new("ctrl-z", crate::kit::Undo, Some("TextInput")),
        ]);

        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);

        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("hello", cx));
        });

        let _ = window.draw(app);
    });

    cx.write_to_clipboard(ClipboardItem::new_string(" world".to_string()));
    cx.simulate_keystrokes("ctrl-v");
    let text = cx.update(|_window, app| view.read(app).input.read(app).text().to_string());
    assert_eq!(text, "hello world");

    cx.simulate_keystrokes("ctrl-z");
    let text = cx.update(|_window, app| view.read(app).input.read(app).text().to_string());
    assert_eq!(text, "hello");
}

#[gpui::test]
fn text_input_double_click_selects_word(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(SmokeView::new);

    cx.update(|window, app| {
        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);

        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("alpha beta", cx));
        });

        let _ = window.draw(app);
    });

    let bounds = cx
        .debug_bounds("smoke_input")
        .expect("expected smoke input bounds");
    let click = cx.update(|_window, app| {
        let input = view.read(app).input.clone();
        (0..200usize)
            .find_map(|step| {
                let pos = gpui::point(bounds.left() + px(8.0 + step as f32), bounds.center().y);
                let offset = input.read(app).offset_for_position(pos);
                (2..=4).contains(&offset).then_some(pos)
            })
            .unwrap_or_else(|| bounds.center())
    });

    cx.simulate_mouse_move(click, None, Modifiers::default());
    cx.simulate_event(MouseDownEvent {
        position: click,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: click,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 2,
    });

    let selection = cx.update(|_window, app| view.read(app).input.read(app).selected_text());
    assert_eq!(selection, Some("alpha".into()));
}

#[gpui::test]
fn text_input_supports_shift_home_end_row_selection(cx: &mut gpui::TestAppContext) {
    let _clipboard_guard = lock_clipboard_test();
    let (view, cx) = cx.add_window_view(SmokeView::new);

    cx.update(|window, app| {
        app.bind_keys([
            KeyBinding::new("left", crate::kit::Left, Some("TextInput")),
            KeyBinding::new("right", crate::kit::Right, Some("TextInput")),
            KeyBinding::new("shift-home", crate::kit::SelectHome, Some("TextInput")),
            KeyBinding::new("shift-end", crate::kit::SelectEnd, Some("TextInput")),
            KeyBinding::new("ctrl-c", crate::kit::Copy, Some("TextInput")),
        ]);

        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);

        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("abcde\n12345", cx));
        });

        let _ = window.draw(app);
    });

    cx.simulate_keystrokes("left left shift-home ctrl-c");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("123".into())
    );

    cx.simulate_keystrokes("right shift-end ctrl-c");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("45".into())
    );
}

#[gpui::test]
fn text_input_supports_shift_pageup_pagedown_selection(cx: &mut gpui::TestAppContext) {
    let _clipboard_guard = lock_clipboard_test();
    let (view, cx) = cx.add_window_view(SmokeView::new);

    cx.update(|window, app| {
        app.bind_keys([
            KeyBinding::new("home", crate::kit::Home, Some("TextInput")),
            KeyBinding::new("left", crate::kit::Left, Some("TextInput")),
            KeyBinding::new("right", crate::kit::Right, Some("TextInput")),
            KeyBinding::new("shift-pageup", crate::kit::SelectPageUp, Some("TextInput")),
            KeyBinding::new(
                "shift-pagedown",
                crate::kit::SelectPageDown,
                Some("TextInput"),
            ),
            KeyBinding::new("ctrl-c", crate::kit::Copy, Some("TextInput")),
        ]);

        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);

        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("abcde\n12345\nxyz", cx));
        });

        let _ = window.draw(app);
    });

    // Move the cursor to the start of the second line.
    cx.simulate_keystrokes("home left home");

    cx.simulate_keystrokes("shift-pageup ctrl-c");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("abcde\n".into())
    );

    // Collapse selection back to the start of the second line.
    cx.simulate_keystrokes("right");

    cx.simulate_keystrokes("shift-pagedown ctrl-c");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("12345\n".into())
    );
}

#[gpui::test]
fn text_input_supports_up_down_with_sticky_column(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(SmokeView::new);

    cx.update(|window, app| {
        app.bind_keys([
            KeyBinding::new("left", crate::kit::Left, Some("TextInput")),
            KeyBinding::new("up", crate::kit::Up, Some("TextInput")),
            KeyBinding::new("down", crate::kit::Down, Some("TextInput")),
        ]);

        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);

        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("aaaaa\nbb\nccccc", cx));
        });

        let _ = window.draw(app);
    });

    // Cursor starts at EOF (offset 14). Move to column 4 on the third line.
    cx.simulate_keystrokes("left");
    let offset = cx.update(|_window, app| view.read(app).input.read(app).cursor_offset());
    assert_eq!(offset, 13);

    // Move up onto shorter middle line, then keep sticky column when moving again.
    cx.simulate_keystrokes("up");
    let offset = cx.update(|_window, app| view.read(app).input.read(app).cursor_offset());
    assert_eq!(offset, 8);

    cx.simulate_keystrokes("up");
    let offset = cx.update(|_window, app| view.read(app).input.read(app).cursor_offset());
    assert_eq!(offset, 4);

    cx.simulate_keystrokes("down down");
    let offset = cx.update(|_window, app| view.read(app).input.read(app).cursor_offset());
    assert_eq!(offset, 13);
}

#[gpui::test]
fn text_input_supports_shift_up_down_selection(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(SmokeView::new);

    cx.update(|window, app| {
        app.bind_keys([
            KeyBinding::new("home", crate::kit::Home, Some("TextInput")),
            KeyBinding::new("left", crate::kit::Left, Some("TextInput")),
            KeyBinding::new("right", crate::kit::Right, Some("TextInput")),
        ]);

        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);

        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("abcde\n12345\nxyz", cx));
        });

        let _ = window.draw(app);
    });

    // Move the cursor to the start of the second line.
    cx.simulate_keystrokes("home left home");
    cx.dispatch_action(crate::kit::SelectUp);
    let selection = cx.update(|_window, app| view.read(app).input.read(app).selected_text());
    assert_eq!(selection, Some("abcde\n".into()));

    // Collapse selection to the start of the second line.
    cx.simulate_keystrokes("right");
    cx.dispatch_action(crate::kit::SelectDown);
    let selection = cx.update(|_window, app| view.read(app).input.read(app).selected_text());
    assert_eq!(selection, Some("12345\n".into()));
}

struct TestBackend;

impl GitBackend for TestBackend {
    fn open(&self, _workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        Err(Error::new(ErrorKind::Unsupported(
            "Test backend does not open repositories",
        )))
    }
}

struct SlowSubmoduleBackend;

impl GitBackend for SlowSubmoduleBackend {
    fn open(&self, workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        Ok(Arc::new(SlowSubmoduleRepo {
            spec: RepoSpec {
                workdir: workdir.to_path_buf(),
            },
        }))
    }
}

struct SlowSubmoduleRepo {
    spec: RepoSpec,
}

impl SlowSubmoduleRepo {
    fn unsupported<T>() -> Result<T> {
        Err(Error::new(ErrorKind::Unsupported(
            "Slow submodule test repo does not implement this operation",
        )))
    }
}

impl GitRepository for SlowSubmoduleRepo {
    fn spec(&self) -> &RepoSpec {
        &self.spec
    }

    fn log_head_page(&self, _limit: usize, _cursor: Option<&LogCursor>) -> Result<LogPage> {
        Self::unsupported()
    }

    fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
        Self::unsupported()
    }

    fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
        Self::unsupported()
    }

    fn current_branch(&self) -> Result<String> {
        Self::unsupported()
    }

    fn list_branches(&self) -> Result<Vec<Branch>> {
        Self::unsupported()
    }

    fn list_remotes(&self) -> Result<Vec<Remote>> {
        Self::unsupported()
    }

    fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
        Self::unsupported()
    }

    fn status(&self) -> Result<RepoStatus> {
        Self::unsupported()
    }

    fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
        Self::unsupported()
    }

    fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
        Self::unsupported()
    }

    fn delete_branch(&self, _name: &str) -> Result<()> {
        Self::unsupported()
    }

    fn checkout_branch(&self, _name: &str) -> Result<()> {
        Self::unsupported()
    }

    fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
        Self::unsupported()
    }

    fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
        Self::unsupported()
    }

    fn revert(&self, _id: &CommitId) -> Result<()> {
        Self::unsupported()
    }

    fn list_submodules(&self) -> Result<Vec<Submodule>> {
        std::thread::sleep(Duration::from_millis(250));
        Ok(Vec::new())
    }

    fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
        Self::unsupported()
    }

    fn stash_list(&self) -> Result<Vec<StashEntry>> {
        Self::unsupported()
    }

    fn stash_apply(&self, _index: usize) -> Result<()> {
        Self::unsupported()
    }

    fn stash_drop(&self, _index: usize) -> Result<()> {
        Self::unsupported()
    }

    fn stage(&self, _paths: &[&Path]) -> Result<()> {
        Self::unsupported()
    }

    fn unstage(&self, _paths: &[&Path]) -> Result<()> {
        Self::unsupported()
    }

    fn commit(&self, _message: &str) -> Result<()> {
        Self::unsupported()
    }

    fn fetch_all(&self) -> Result<()> {
        Self::unsupported()
    }

    fn pull(&self, _mode: PullMode) -> Result<()> {
        Self::unsupported()
    }

    fn push(&self) -> Result<()> {
        Self::unsupported()
    }

    fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
        Self::unsupported()
    }
}

struct SlowStashBackend;

impl GitBackend for SlowStashBackend {
    fn open(&self, workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        Ok(Arc::new(SlowStashRepo {
            spec: RepoSpec {
                workdir: workdir.to_path_buf(),
            },
        }))
    }
}

struct SlowStashRepo {
    spec: RepoSpec,
}

impl SlowStashRepo {
    fn unsupported<T>() -> Result<T> {
        Err(Error::new(ErrorKind::Unsupported(
            "Slow stash test repo does not implement this operation",
        )))
    }
}

impl GitRepository for SlowStashRepo {
    fn spec(&self) -> &RepoSpec {
        &self.spec
    }

    fn log_head_page(&self, _limit: usize, _cursor: Option<&LogCursor>) -> Result<LogPage> {
        Self::unsupported()
    }

    fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
        Self::unsupported()
    }

    fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
        Self::unsupported()
    }

    fn current_branch(&self) -> Result<String> {
        Self::unsupported()
    }

    fn list_branches(&self) -> Result<Vec<Branch>> {
        Self::unsupported()
    }

    fn list_remotes(&self) -> Result<Vec<Remote>> {
        Self::unsupported()
    }

    fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
        Self::unsupported()
    }

    fn status(&self) -> Result<RepoStatus> {
        Self::unsupported()
    }

    fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
        Self::unsupported()
    }

    fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
        Self::unsupported()
    }

    fn delete_branch(&self, _name: &str) -> Result<()> {
        Self::unsupported()
    }

    fn checkout_branch(&self, _name: &str) -> Result<()> {
        Self::unsupported()
    }

    fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
        Self::unsupported()
    }

    fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
        Self::unsupported()
    }

    fn revert(&self, _id: &CommitId) -> Result<()> {
        Self::unsupported()
    }

    fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
        Self::unsupported()
    }

    fn stash_list(&self) -> Result<Vec<StashEntry>> {
        std::thread::sleep(Duration::from_millis(250));
        Ok(Vec::new())
    }

    fn stash_apply(&self, _index: usize) -> Result<()> {
        Self::unsupported()
    }

    fn stash_drop(&self, _index: usize) -> Result<()> {
        Self::unsupported()
    }

    fn stage(&self, _paths: &[&Path]) -> Result<()> {
        Self::unsupported()
    }

    fn unstage(&self, _paths: &[&Path]) -> Result<()> {
        Self::unsupported()
    }

    fn commit(&self, _message: &str) -> Result<()> {
        Self::unsupported()
    }

    fn fetch_all(&self) -> Result<()> {
        Self::unsupported()
    }

    fn pull(&self, _mode: PullMode) -> Result<()> {
        Self::unsupported()
    }

    fn push(&self) -> Result<()> {
        Self::unsupported()
    }

    fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
        Self::unsupported()
    }
}

fn repo_tab_selector(repo_id: RepoId) -> &'static str {
    Box::leak(format!("repo_tab_{}", repo_id.0).into_boxed_str())
}

fn worktrees_spinner_selector(repo_id: RepoId) -> &'static str {
    Box::leak(format!("worktrees_spinner_{}", repo_id.0).into_boxed_str())
}

fn submodules_spinner_selector(repo_id: RepoId) -> &'static str {
    Box::leak(format!("submodules_spinner_{}", repo_id.0).into_boxed_str())
}

fn stash_spinner_selector(repo_id: RepoId) -> &'static str {
    Box::leak(format!("stash_spinner_{}", repo_id.0).into_boxed_str())
}

fn debug_selector(prefix: &str, ix: usize) -> &'static str {
    Box::leak(format!("{prefix}_{ix}").into_boxed_str())
}

fn wait_for_repo_count(store: &AppStore, expected: usize) -> Arc<gitcomet_state::model::AppState> {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let state = store.snapshot();
        if state.repos.len() == expected {
            return state;
        }
        if Instant::now() >= deadline {
            panic!(
                "timed out waiting for store repos len {expected}, got {}",
                state.repos.len()
            );
        }
        std::thread::yield_now();
    }
}

fn wait_for_repo_order(store: &AppStore, expected: &[RepoId]) {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let state = store.snapshot();
        let got = state.repos.iter().map(|r| r.id).collect::<Vec<_>>();
        if got == expected {
            return;
        }
        if Instant::now() >= deadline {
            panic!("timed out waiting for repo order {expected:?}, got {got:?}");
        }
        std::thread::yield_now();
    }
}

fn wait_for_repo_open(store: &AppStore, repo_id: RepoId) {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let state = store.snapshot();
        if state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .is_some_and(|repo| matches!(repo.open, Loadable::Ready(())))
        {
            return;
        }
        if Instant::now() >= deadline {
            panic!("timed out waiting for repo {repo_id:?} to open");
        }
        std::thread::yield_now();
    }
}

fn wait_until(description: &str, ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        if ready() {
            return;
        }
        if Instant::now() >= deadline {
            panic!("timed out waiting for {description}");
        }
        std::thread::yield_now();
    }
}

fn seed_workspace_repo(
    cx: &mut gpui::VisualTestContext,
    store: &AppStore,
    view: gpui::Entity<crate::view::GitCometView>,
    path: PathBuf,
) {
    store.dispatch(Msg::OpenRepo(path));

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                crate::view::test_support::sync_store_snapshot(this, cx)
            });
            let _ = window.draw(app);
        });
        cx.run_until_parked();

        let ready = cx.update(|_window, app| !view.read(app).blocks_non_repository_actions());
        if ready {
            return;
        }

        if Instant::now() >= deadline {
            panic!("timed out waiting for the workspace view to leave the splash state");
        }

        std::thread::sleep(Duration::from_millis(10));
    }
}

fn sync_view_for_tests(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<crate::view::GitCometView>,
) {
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx)
        });
        let _ = window.draw(app);
    });
}

fn find_debug_index(
    cx: &mut gpui::VisualTestContext,
    prefix: &str,
    max_ix: usize,
) -> Option<usize> {
    (0..max_ix).find(|ix| cx.debug_bounds(debug_selector(prefix, *ix)).is_some())
}

fn wait_for_debug_index(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<crate::view::GitCometView>,
    prefix: &str,
    max_ix: usize,
) -> usize {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        sync_view_for_tests(cx, view);

        if let Some(ix) = find_debug_index(cx, prefix, max_ix) {
            return ix;
        }

        if Instant::now() >= deadline {
            panic!("timed out waiting for debug selector prefix {prefix}");
        }

        cx.run_until_parked();
        std::thread::yield_now();
    }
}

fn click_debug_selector(
    cx: &mut gpui::VisualTestContext,
    selector: &'static str,
    click_count: usize,
) {
    click_debug_selector_with_button(cx, selector, MouseButton::Left, click_count);
}

fn click_debug_selector_with_button(
    cx: &mut gpui::VisualTestContext,
    selector: &'static str,
    button: MouseButton,
    click_count: usize,
) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("expected debug selector {selector}"));
    let center = bounds.center();
    cx.simulate_mouse_move(center, None, Modifiers::default());
    cx.simulate_event(MouseDownEvent {
        position: center,
        modifiers: Modifiers::default(),
        button,
        click_count,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: center,
        modifiers: Modifiers::default(),
        button,
        click_count,
    });
}

fn restore_session_and_draw(
    cx: &mut gpui::VisualTestContext,
    store: &AppStore,
    view: gpui::Entity<crate::view::GitCometView>,
    repos: Vec<PathBuf>,
) -> Vec<RepoId> {
    store.dispatch(Msg::RestoreSession {
        open_repos: repos.clone(),
        active_repo: repos.first().cloned(),
    });

    let state = wait_for_repo_count(store, repos.len());
    let ids = state.repos.iter().map(|r| r.id).collect::<Vec<_>>();
    let selectors = ids
        .iter()
        .copied()
        .map(repo_tab_selector)
        .collect::<Vec<_>>();

    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        sync_view_for_tests(cx, &view);

        if selectors
            .iter()
            .all(|selector| cx.debug_bounds(selector).is_some())
        {
            return ids;
        }

        if Instant::now() >= deadline {
            panic!(
                "timed out waiting for repo tabs to render; missing selectors: {:?}",
                selectors
                    .into_iter()
                    .filter(|selector| cx.debug_bounds(selector).is_none())
                    .collect::<Vec<_>>()
            );
        }

        cx.run_until_parked();
        std::thread::yield_now();
    }
}

#[gpui::test]
fn gitcomet_view_renders_without_panicking(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        let (store, events) = AppStore::new(Arc::new(TestBackend));
        cx.open_window(Default::default(), |window, cx| {
            cx.new(|cx| crate::view::GitCometView::new(store, events, None, window, cx))
        })
        .unwrap();
    });
}

#[gpui::test]
fn repo_tabs_can_drag_reorder_by_right_half(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_test = store.clone();
    let (_view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_repo_tabs_right_{}",
        std::process::id()
    ));
    let repo_ids = restore_session_and_draw(
        cx,
        &store_for_test,
        _view.clone(),
        vec![base.join("repo1"), base.join("repo2"), base.join("repo3")],
    );

    let dragged = repo_ids[0];
    let target = repo_ids[1];
    let expected = vec![repo_ids[1], repo_ids[0], repo_ids[2]];

    let dragged_bounds = cx
        .debug_bounds(repo_tab_selector(dragged))
        .expect("expected dragged repo tab bounds");
    let target_bounds = cx
        .debug_bounds(repo_tab_selector(target))
        .expect("expected target repo tab bounds");

    let start = dragged_bounds.center();
    cx.simulate_mouse_move(start, None, Modifiers::default());
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        gpui::point(start.x + px(10.0), start.y),
        Some(MouseButton::Left),
        Modifiers::default(),
    );

    let drop = gpui::point(target_bounds.right() - px(5.0), target_bounds.center().y);
    cx.simulate_mouse_move(drop, Some(MouseButton::Left), Modifiers::default());
    cx.simulate_mouse_up(drop, MouseButton::Left, Modifiers::default());

    wait_for_repo_order(&store_for_test, &expected);
}

#[gpui::test]
fn repo_tabs_can_drag_reorder_by_left_half(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_test = store.clone();
    let (_view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_repo_tabs_left_{}",
        std::process::id()
    ));
    let repo_ids = restore_session_and_draw(
        cx,
        &store_for_test,
        _view.clone(),
        vec![base.join("repo1"), base.join("repo2"), base.join("repo3")],
    );

    let dragged = repo_ids[2];
    let target = repo_ids[1];
    let expected = vec![repo_ids[0], repo_ids[2], repo_ids[1]];

    let dragged_bounds = cx
        .debug_bounds(repo_tab_selector(dragged))
        .expect("expected dragged repo tab bounds");
    let target_bounds = cx
        .debug_bounds(repo_tab_selector(target))
        .expect("expected target repo tab bounds");

    let start = dragged_bounds.center();
    cx.simulate_mouse_move(start, None, Modifiers::default());
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        gpui::point(start.x - px(10.0), start.y),
        Some(MouseButton::Left),
        Modifiers::default(),
    );

    let drop = gpui::point(target_bounds.left() + px(5.0), target_bounds.center().y);
    cx.simulate_mouse_move(drop, Some(MouseButton::Left), Modifiers::default());
    cx.simulate_mouse_up(drop, MouseButton::Left, Modifiers::default());

    wait_for_repo_order(&store_for_test, &expected);
}

#[gpui::test]
fn repo_tabs_drop_on_self_is_noop(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_test = store.clone();
    let (_view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_repo_tabs_self_{}",
        std::process::id()
    ));
    let repo_ids = restore_session_and_draw(
        cx,
        &store_for_test,
        _view.clone(),
        vec![base.join("repo1"), base.join("repo2"), base.join("repo3")],
    );

    let dragged = repo_ids[1];
    let dragged_bounds = cx
        .debug_bounds(repo_tab_selector(dragged))
        .expect("expected dragged repo tab bounds");

    let start = dragged_bounds.center();
    cx.simulate_mouse_move(start, None, Modifiers::default());
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());

    let moved_x = (start.x + px(10.0)).min(dragged_bounds.right() - px(1.0));
    let moved = gpui::point(moved_x, start.y);
    cx.simulate_mouse_move(moved, Some(MouseButton::Left), Modifiers::default());
    cx.simulate_mouse_up(moved, MouseButton::Left, Modifiers::default());

    let got = store_for_test
        .snapshot()
        .repos
        .iter()
        .map(|r| r.id)
        .collect::<Vec<_>>();
    assert_eq!(got, repo_ids);
}

#[gpui::test]
fn repo_tabs_middle_click_closes_inactive_tab_without_reactivating(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_test = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_repo_tabs_middle_close_{}",
        std::process::id()
    ));
    let repo_ids = restore_session_and_draw(
        cx,
        &store_for_test,
        view.clone(),
        vec![base.join("repo1"), base.join("repo2"), base.join("repo3")],
    );

    let active_repo = repo_ids[0];
    let closed_repo = repo_ids[1];
    click_debug_selector_with_button(cx, repo_tab_selector(closed_repo), MouseButton::Middle, 1);

    let state = wait_for_repo_count(&store_for_test, 2);
    assert_eq!(
        state.repos.iter().map(|repo| repo.id).collect::<Vec<_>>(),
        vec![repo_ids[0], repo_ids[2]]
    );
    assert_eq!(state.active_repo, Some(active_repo));

    sync_view_for_tests(cx, &view);
    assert!(
        cx.debug_bounds(repo_tab_selector(closed_repo)).is_none(),
        "expected middle-clicked repo tab to be removed from the UI"
    );
}

#[gpui::test]
fn repo_tabs_right_click_does_not_close(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_test = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_repo_tabs_right_click_{}",
        std::process::id()
    ));
    let repo_ids = restore_session_and_draw(
        cx,
        &store_for_test,
        view.clone(),
        vec![base.join("repo1"), base.join("repo2"), base.join("repo3")],
    );

    click_debug_selector_with_button(cx, repo_tab_selector(repo_ids[1]), MouseButton::Right, 1);

    let state = store_for_test.snapshot();
    assert_eq!(
        state.repos.iter().map(|repo| repo.id).collect::<Vec<_>>(),
        repo_ids
    );
    assert_eq!(state.active_repo, Some(repo_ids[0]));

    sync_view_for_tests(cx, &view);
    assert!(
        cx.debug_bounds(repo_tab_selector(repo_ids[1])).is_some(),
        "expected right-clicked repo tab to remain visible"
    );
}

#[gpui::test]
fn worktrees_section_shows_spinner_while_removing_worktree(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_test = store.clone();
    let (_view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_worktrees_spinner_{}",
        std::process::id()
    ));
    let repo_ids =
        restore_session_and_draw(cx, &store_for_test, _view.clone(), vec![base.join("repo1")]);
    let repo_id = repo_ids[0];

    store_for_test.dispatch(Msg::RemoveWorktree {
        repo_id,
        path: base.join("repo1").join("worktree_to_remove"),
    });

    let selector = worktrees_spinner_selector(repo_id);
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        sync_view_for_tests(cx, &_view);

        if cx.debug_bounds(selector).is_some() {
            break;
        }

        if Instant::now() >= deadline {
            panic!("timed out waiting for worktrees spinner to render");
        }

        cx.run_until_parked();
        std::thread::yield_now();
    }
}

#[gpui::test]
fn submodules_section_shows_spinner_while_loading(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(SlowSubmoduleBackend));
    let store_for_test = store.clone();
    let (_view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_submodules_spinner_{}",
        std::process::id()
    ));
    let repo_ids =
        restore_session_and_draw(cx, &store_for_test, _view.clone(), vec![base.join("repo1")]);
    let repo_id = repo_ids[0];
    wait_for_repo_open(&store_for_test, repo_id);

    let section_ix = wait_for_debug_index(cx, &_view, "submodules_section", 64);
    let section_selector = debug_selector("submodules_section", section_ix);
    click_debug_selector(cx, section_selector, 1);

    let selector = submodules_spinner_selector(repo_id);
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        sync_view_for_tests(cx, &_view);

        let repo_requested = store_for_test
            .snapshot()
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .is_some_and(|repo| {
                repo.sidebar_data_request.submodules && matches!(repo.submodules, Loadable::Loading)
            });

        if repo_requested && cx.debug_bounds(selector).is_some() {
            break;
        }

        if Instant::now() >= deadline {
            panic!("timed out waiting for submodules spinner to render");
        }

        cx.run_until_parked();
        std::thread::yield_now();
    }
}

#[gpui::test]
fn stash_section_shows_spinner_while_loading(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(SlowStashBackend));
    let store_for_test = store.clone();
    let (_view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_stash_spinner_{}",
        std::process::id()
    ));
    let repo_ids =
        restore_session_and_draw(cx, &store_for_test, _view.clone(), vec![base.join("repo1")]);
    let repo_id = repo_ids[0];
    wait_for_repo_open(&store_for_test, repo_id);

    let section_ix = wait_for_debug_index(cx, &_view, "stash_section", 64);
    let section_selector = debug_selector("stash_section", section_ix);
    click_debug_selector(cx, section_selector, 1);

    let selector = stash_spinner_selector(repo_id);
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        sync_view_for_tests(cx, &_view);

        let repo_requested = store_for_test
            .snapshot()
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .is_some_and(|repo| {
                repo.sidebar_data_request.stashes && matches!(repo.stashes, Loadable::Loading)
            });

        if repo_requested && cx.debug_bounds(selector).is_some() {
            break;
        }

        if Instant::now() >= deadline {
            panic!("timed out waiting for stash spinner to render");
        }

        cx.run_until_parked();
        std::thread::yield_now();
    }
}

#[gpui::test]
fn listed_workspace_badge_double_click_opens_closed_repo_tab(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(SlowSubmoduleBackend));
    let store_for_test = store.clone();
    let (_view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_badge_open_{}",
        std::process::id()
    ));
    let repo_ids =
        restore_session_and_draw(cx, &store_for_test, _view.clone(), vec![base.join("repo1")]);
    let repo_id = repo_ids[0];
    wait_for_repo_open(&store_for_test, repo_id);

    let linked_repo = base.join("repo-feature");
    store_for_test.dispatch(Msg::Internal(
        gitcomet_state::msg::InternalMsg::BranchesLoaded {
            repo_id,
            result: Ok(vec![Branch {
                name: "feature/workspace".to_string(),
                target: CommitId("deadbeef".into()),
                upstream: None,
                divergence: None,
            }]),
        },
    ));
    store_for_test.dispatch(Msg::Internal(
        gitcomet_state::msg::InternalMsg::WorktreesLoaded {
            repo_id,
            result: Ok(vec![Worktree {
                path: linked_repo.clone(),
                head: None,
                branch: Some("feature/workspace".to_string()),
                detached: false,
            }]),
        },
    ));

    let badge_ix = wait_for_debug_index(cx, &_view, "branch_workspace_badge", 64);
    let badge_selector = debug_selector("branch_workspace_badge", badge_ix);
    click_debug_selector(cx, badge_selector, 2);

    wait_until("linked repository tab to open from badge", || {
        let snapshot = store_for_test.snapshot();
        snapshot
            .repos
            .iter()
            .any(|repo| repo.spec.workdir == linked_repo)
            && snapshot
                .active_repo
                .and_then(|active_repo| snapshot.repos.iter().find(|repo| repo.id == active_repo))
                .is_some_and(|repo| repo.spec.workdir == linked_repo)
    });
}

struct PanelLayoutTestView {
    theme: AppTheme,
    handle: gpui::UniformListScrollHandle,
}

impl PanelLayoutTestView {
    fn new() -> Self {
        Self {
            theme: AppTheme::gitcomet_dark(),
            handle: gpui::UniformListScrollHandle::default(),
        }
    }
}

impl gpui::Render for PanelLayoutTestView {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let theme = self.theme;

        let header = div().id("diff_header").h(px(24.0)).child("Header");
        let list = gpui::uniform_list(
            "diff_list",
            200,
            cx.processor(
                |_this: &mut PanelLayoutTestView,
                 range: std::ops::Range<usize>,
                 _window: &mut gpui::Window,
                 _cx: &mut gpui::Context<PanelLayoutTestView>| {
                    range
                        .map(|ix| {
                            div()
                                .id(ix)
                                .h(px(20.0))
                                .px_2()
                                .child(format!("Row {ix}"))
                                .into_any_element()
                        })
                        .collect::<Vec<_>>()
                },
            ),
        )
        .h_full()
        .track_scroll(&self.handle);

        let body = div()
            .id("diff_body")
            .debug_selector(|| "diff_body".to_string())
            .flex()
            .flex_col()
            .h_full()
            .child(header)
            .child({
                let scrollbar =
                    components::Scrollbar::new("diff_scrollbar_test", self.handle.clone());
                #[cfg(test)]
                let scrollbar = scrollbar.debug_selector("diff_scrollbar_test");

                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .relative()
                    .child(list)
                    .child(scrollbar.render(theme))
            });

        div().size_full().bg(theme.colors.window_bg).child(
            components::panel(theme, "Panel", None, body)
                .flex_1()
                .h_full(),
        )
    }
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
                    multiline: false,
                    read_only: false,
                    chromeless: false,
                    soft_wrap: false,
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
        let items = (0..50)
            .map(|ix| SharedString::from(format!("Commit {ix:02}  Synthetic history entry")))
            .collect::<Vec<_>>();

        div().size_full().bg(self.theme.colors.window_bg).child(
            div().w(px(360.0)).child(
                components::PickerPrompt::new(self.input.clone(), self.scroll_handle.clone())
                    .items(items)
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
fn popover_is_clickable_above_content(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store_for_view, events, None, window, cx)
    });
    seed_workspace_repo(
        cx,
        &store,
        view.clone(),
        PathBuf::from("/tmp/gitcomet-smoke-popover-click-test"),
    );

    // Open the repo picker dropdown in the action bar, which should overlay the rest of the UI.
    let picker_bounds = cx
        .debug_bounds("repo_picker")
        .expect("expected repo_picker in debug bounds");
    cx.simulate_mouse_move(picker_bounds.center(), None, Modifiers::default());
    cx.simulate_mouse_down(
        picker_bounds.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        picker_bounds.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let close_bounds = cx
        .debug_bounds("repo_popover_close")
        .expect("expected repo_popover_close in debug bounds");
    cx.simulate_mouse_move(close_bounds.center(), None, Modifiers::default());
    cx.simulate_mouse_down(
        close_bounds.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        close_bounds.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        assert!(
            !crate::view::test_support::popover_is_open(view.read(app), app),
            "expected popover to close on click"
        );
    });
}

#[gpui::test]
fn popover_closes_when_clicking_outside(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store_for_view, events, None, window, cx)
    });
    seed_workspace_repo(
        cx,
        &store,
        view.clone(),
        PathBuf::from("/tmp/gitcomet-smoke-popover-outside-test"),
    );

    let picker_bounds = cx
        .debug_bounds("repo_picker")
        .expect("expected repo_picker in debug bounds");
    cx.simulate_mouse_move(picker_bounds.center(), None, Modifiers::default());
    cx.simulate_mouse_down(
        picker_bounds.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        picker_bounds.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.run_until_parked();

    cx.update(|_window, app| {
        assert!(
            crate::view::test_support::popover_is_open(view.read(app), app),
            "expected popover to open"
        );
    });

    // Click somewhere in the main content area (outside the popover).
    let outside = gpui::point(px(900.0), px(700.0));
    cx.simulate_mouse_move(outside, None, Modifiers::default());
    cx.simulate_mouse_down(outside, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(outside, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();

    cx.update(|_window, app| {
        assert!(
            !crate::view::test_support::popover_is_open(view.read(app), app),
            "expected popover to close when clicking outside"
        );
    });
}

#[gpui::test]
fn titlebar_hamburger_opens_app_menu_but_brand_pill_does_not(cx: &mut gpui::TestAppContext) {
    if cfg!(target_os = "macos") {
        return;
    }

    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store_for_view, events, None, window, cx)
    });
    seed_workspace_repo(
        cx,
        &store,
        view.clone(),
        PathBuf::from("/tmp/gitcomet-smoke-titlebar-menu-test"),
    );

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let brand_bounds = cx
        .debug_bounds("titlebar_brand")
        .expect("expected titlebar brand bounds");
    cx.simulate_mouse_move(brand_bounds.center(), None, Modifiers::default());
    cx.simulate_mouse_down(
        brand_bounds.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        brand_bounds.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.update(|_window, app| {
        assert!(
            !crate::view::test_support::popover_is_open(view.read(app), app),
            "expected titlebar brand pill click to leave the app menu closed"
        );
    });

    let menu_bounds = cx
        .debug_bounds("app_menu")
        .expect("expected app menu hamburger bounds");
    cx.simulate_mouse_move(menu_bounds.center(), None, Modifiers::default());
    cx.simulate_mouse_down(
        menu_bounds.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        menu_bounds.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.update(|_window, app| {
        assert!(
            crate::view::test_support::popover_is_open(view.read(app), app),
            "expected hamburger click to open the app menu"
        );
    });
}

#[gpui::test]
fn titlebar_free_badge_opens_editions_page_and_updates_tooltip_on_hover(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let badge_bounds = cx
        .debug_bounds("titlebar_free_badge")
        .expect("expected titlebar free badge bounds");
    let badge_center = badge_bounds.center();

    cx.simulate_mouse_move(badge_center, None, Modifiers::default());
    crate::view::test_support::wait_for_native_tooltip(cx);
    assert_eq!(
        crate::view::test_support::tooltip_text(cx, &view),
        Some("See GitComet editions".into())
    );

    cx.simulate_mouse_down(badge_center, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(badge_center, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();

    assert_eq!(cx.opened_url(), Some(crate::view::EDITIONS_URL.to_string()));
    cx.update(|_window, app| {
        assert!(
            !crate::view::test_support::popover_is_open(view.read(app), app),
            "expected titlebar free badge click to leave popovers closed"
        );
    });

    cx.simulate_mouse_move(gpui::point(px(120.0), px(18.0)), None, Modifiers::default());
    assert_eq!(crate::view::test_support::tooltip_text(cx, &view), None);
}

#[gpui::test]
fn titlebar_free_badge_keeps_the_same_size_across_ui_zoom(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let default_bounds = cx
        .debug_bounds("titlebar_free_badge")
        .expect("expected titlebar free badge bounds at the default zoom");
    let default_width: f32 = default_bounds.size.width.into();
    let default_height: f32 = default_bounds.size.height.into();

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.apply_ui_scale_percent(200, window, cx);
        });
        let _ = window.draw(app);
    });
    cx.run_until_parked();

    let zoomed_bounds = cx
        .debug_bounds("titlebar_free_badge")
        .expect("expected titlebar free badge bounds after zooming");
    let zoomed_width: f32 = zoomed_bounds.size.width.into();
    let zoomed_height: f32 = zoomed_bounds.size.height.into();

    assert!(
        (zoomed_width - default_width).abs() <= 0.5,
        "expected the FREE badge width to stay fixed across zoom (default={default_width}, zoomed={zoomed_width})"
    );
    assert!(
        (zoomed_height - default_height).abs() <= 0.5,
        "expected the FREE badge height to stay fixed across zoom (default={default_height}, zoomed={zoomed_height})"
    );
}

#[gpui::test]
fn titlebar_window_controls_update_tooltip_on_hover(cx: &mut gpui::TestAppContext) {
    if cfg!(target_os = "macos") {
        // The custom Min/Max/Close controls are only rendered on non-macOS.
        return;
    }

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let min_bounds = cx
        .debug_bounds("titlebar_win_min")
        .expect("expected titlebar min control bounds");
    cx.simulate_mouse_move(min_bounds.center(), None, Modifiers::default());
    crate::view::test_support::wait_for_native_tooltip(cx);
    assert_eq!(
        crate::view::test_support::tooltip_text(cx, &view),
        Some("Minimize window".into())
    );

    let max_bounds = cx
        .debug_bounds("titlebar_win_max")
        .expect("expected titlebar max control bounds");
    let expected_max = cx.update(|window, _app| {
        if window.is_maximized() {
            "Restore window".into()
        } else {
            "Maximize window".into()
        }
    });
    cx.simulate_mouse_move(max_bounds.center(), None, Modifiers::default());
    crate::view::test_support::wait_for_native_tooltip(cx);
    assert_eq!(
        crate::view::test_support::tooltip_text(cx, &view),
        Some(expected_max)
    );

    let close_bounds = cx
        .debug_bounds("titlebar_win_close")
        .expect("expected titlebar close control bounds");
    cx.simulate_mouse_move(close_bounds.center(), None, Modifiers::default());
    crate::view::test_support::wait_for_native_tooltip(cx);
    assert_eq!(
        crate::view::test_support::tooltip_text(cx, &view),
        Some("Close window".into())
    );

    cx.simulate_mouse_move(gpui::point(px(120.0), px(18.0)), None, Modifiers::default());
    assert_eq!(crate::view::test_support::tooltip_text(cx, &view), None);
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

        div().size_full().bg(theme.colors.window_bg).child(
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
        div().size_full().bg(theme.colors.window_bg).child(
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
