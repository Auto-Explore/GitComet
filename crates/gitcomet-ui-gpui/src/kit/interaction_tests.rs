use super::*;
use crate::test_support::painted_control_quads as paint;
use crate::view::components::{
    Button, ButtonStyle, InteractiveRowExt, InteractiveRowStyle, SplitButton,
    on_nested_control_click, panel_tab, panel_tab_close,
};
use gpui::{
    Context, FocusHandle, IntoElement, Modifiers, MouseButton, MouseDownEvent, Pixels, Point,
    Render, TestAppContext, VisualTestContext, Window, div,
};

#[derive(Clone, Copy, Debug)]
enum FixtureKind {
    Button(ButtonStyle),
    BoundedButton,
    Row,
    PanelTab,
    Split,
    PaintedRow,
    Menu,
    AppMenu,
    Canvas(MouseButton),
}

struct Fixture {
    theme: AppTheme,
    kind: FixtureKind,
    state: InteractionState,
    explicit_focus: bool,
    before: FocusHandle,
    focus: FocusHandle,
    after: FocusHandle,
    clicks: usize,
    menu_clicks: usize,
    menu_open: bool,
    parent_keys: usize,
    source_clicks: usize,
}

impl Fixture {
    fn new(
        theme: AppTheme,
        kind: FixtureKind,
        explicit_focus: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            theme,
            kind,
            explicit_focus,
            state: InteractionState::default(),
            before: cx.focus_handle().tab_index(0).tab_stop(true),
            focus: cx.focus_handle(),
            after: cx.focus_handle().tab_index(0).tab_stop(true),
            clicks: 0,
            menu_clicks: 0,
            menu_open: false,
            parent_keys: 0,
            source_clicks: 0,
        }
    }

    fn button(&self) -> Button {
        let mut button = Button::new("subject", "Action")
            .style(match self.kind {
                FixtureKind::Button(style) => style,
                _ => ButtonStyle::Subtle,
            })
            .disabled(self.state.disabled)
            .selected(self.state.selected.is_some())
            .open(self.state.open)
            .busy(self.state.busy);
        if self.explicit_focus {
            button = button.focus_handle(self.focus.clone());
        }
        button
    }
}

impl Render for Fixture {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let subject = match self.kind {
            FixtureKind::Canvas(button) => {
                let view = cx.entity();
                div()
                    .id("subject")
                    .debug_selector(|| "subject".into())
                    .w(px(100.0))
                    .h(px(28.0))
                    .child(
                        gpui::canvas(
                            |bounds, window, _| {
                                window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal)
                            },
                            move |_, hitbox, window, _| {
                                let view = view.clone();
                                crate::kit::click::on_canvas_click(
                                    window,
                                    "subject".into(),
                                    &hitbox,
                                    button,
                                    true,
                                    move |_, _, app| {
                                        view.update(app, |this, _| this.clicks += 1);
                                    },
                                );
                            },
                        )
                        .size_full(),
                    )
                    .into_any_element()
            }
            FixtureKind::PaintedRow => {
                let paint = crate::kit::interaction_paint::InteractionPaint::new(
                    InteractionStyle::new(theme).on_surface(theme.colors.surface.chrome),
                    self.state,
                );
                paint
                    .apply(
                        div()
                            .id("subject")
                            .debug_selector(|| "subject".to_string())
                            .w(px(100.0))
                            .h(px(28.0))
                            .flex(),
                    )
                    .child(
                        div()
                            .id("painted_mask")
                            .debug_selector(|| "painted_mask".to_string())
                            .w(px(12.0))
                            .h_full()
                            .child(
                                gpui::canvas(
                                    |_, _, _| (),
                                    move |bounds, _, window, _| {
                                        window.paint_quad(gpui::fill(
                                            bounds,
                                            paint.background(theme.colors.surface.chrome, window),
                                        ));
                                    },
                                )
                                .size_full(),
                            ),
                    )
                    .child("Row label")
                    .into_any_element()
            }
            FixtureKind::Menu => {
                crate::kit::menu::menu_item("subject", theme, 100, false, self.state.disabled)
                    .debug_selector(|| "subject".to_string())
                    .w(px(100.0))
                    .child("Menu action")
                    .on_menu_activate(
                        self.state.disabled,
                        cx.listener(|this, _, _, _| this.clicks += 1),
                    )
                    .into_any_element()
            }
            FixtureKind::AppMenu => crate::view::components::ContextMenuEntry::new(
                "subject",
                crate::view::components::ContextMenuText::new("Menu action"),
            )
            .disabled(self.state.disabled)
            .on_select(theme, 100, cx, |this, _, _, _| this.clicks += 1)
            .debug_selector(|| "subject".to_string())
            .w(px(100.0))
            .into_any_element(),
            FixtureKind::Button(_) => self
                .button()
                .on_click(theme, cx, |this, _, _, _| this.clicks += 1)
                .into_any_element(),
            FixtureKind::BoundedButton => self
                .button()
                .on_click_with_bounds(theme, cx, |this, _, bounds, _, _| {
                    assert!(bounds.size.width > px(0.0));
                    this.clicks += 1;
                })
                .into_any_element(),
            FixtureKind::Row => div()
                .id("subject")
                .debug_selector(|| "subject".to_string())
                .w(px(100.0))
                .h(px(28.0))
                .track_focus(&self.focus)
                .interactive_row(
                    InteractiveRowStyle::new(theme, theme.colors.surface.chrome),
                    self.state,
                )
                .child("Row")
                .on_activate(
                    false,
                    ControlActivation::Composite,
                    cx.listener(|this, _, _, _| this.clicks += 1),
                )
                .into_any_element(),
            FixtureKind::PanelTab => {
                let close = on_nested_control_click(
                    panel_tab_close("close", theme, 100, theme.colors.foreground.secondary),
                    cx,
                    |this, _, _, _| this.menu_clicks += 1,
                );
                panel_tab(
                    "subject",
                    theme,
                    100,
                    "icons/terminal.svg",
                    "Tab",
                    self.state.selected.is_some(),
                )
                .track_focus(&self.focus)
                .child(close)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, _| this.clicks += 1),
                )
                .into_any_element()
            }
            FixtureKind::Split => SplitButton::action_menu(
                self.button(),
                Button::new("menu", "v").open(self.menu_open),
                theme,
                cx,
                |this, _, _, _| this.clicks += 1,
                |this, _, _, _, cx| {
                    this.menu_clicks += 1;
                    this.menu_open = true;
                    cx.notify();
                },
            )
            .render(theme, 100)
            .into_any_element(),
        };
        div()
            .tab_group()
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, _| {
                if matches!(event.keystroke.key.as_str(), "space" | "enter") {
                    this.parent_keys += 1;
                }
            }))
            .on_key_up(cx.listener(|this, event: &gpui::KeyUpEvent, _, _| {
                if matches!(event.keystroke.key.as_str(), "space" | "enter") {
                    this.parent_keys += 1;
                }
            }))
            .w(px(300.0))
            .h(px(220.0))
            .bg(theme.colors.surface.chrome)
            .flex()
            .flex_col()
            .items_start()
            .gap(px(8.0))
            .p(px(12.0))
            .child(
                div()
                    .id("before")
                    .debug_selector(|| "before".to_string())
                    .on_activate(
                        false,
                        ControlActivation::ManagedFocus,
                        cx.listener(|this, _, _, _| this.source_clicks += 1),
                    )
                    .tab_index(0)
                    .track_focus(&self.before)
                    .child("Before"),
            )
            .child(subject)
            .child(
                div()
                    .id("after")
                    .tab_index(0)
                    .track_focus(&self.after)
                    .child("After"),
            )
    }
}

fn themes() -> [AppTheme; 2] {
    [
        AppTheme::from_key(crate::theme::DEFAULT_DARK_THEME_KEY).unwrap(),
        AppTheme::gitcomet_light(),
    ]
}

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    crate::test_support::refresh_and_draw(cx);
}

fn leave(cx: &mut VisualTestContext) {
    cx.simulate_mouse_move(point(px(280.0), px(200.0)), None, Modifiers::default());
    redraw(cx);
}

fn mouse_down(
    cx: &mut VisualTestContext,
    position: Point<Pixels>,
    button: MouseButton,
    first_mouse: bool,
) {
    // GPUI's simulate_mouse_down and simulate_click always set first_mouse to false.
    cx.simulate_event(MouseDownEvent {
        position,
        button,
        click_count: 1,
        first_mouse,
        ..Default::default()
    });
}

#[gpui::test]
fn focusing_click_activates_controls_and_canvas_once(cx: &mut TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    for (kind, button) in [
        (FixtureKind::Button(ButtonStyle::Subtle), MouseButton::Left),
        (FixtureKind::BoundedButton, MouseButton::Left),
        (FixtureKind::Row, MouseButton::Left),
        (FixtureKind::Split, MouseButton::Left),
        (FixtureKind::Menu, MouseButton::Left),
        (FixtureKind::Menu, MouseButton::Right),
        (FixtureKind::AppMenu, MouseButton::Left),
        (FixtureKind::AppMenu, MouseButton::Right),
        (FixtureKind::Canvas(MouseButton::Left), MouseButton::Left),
        (FixtureKind::Canvas(MouseButton::Right), MouseButton::Right),
    ] {
        let (view, cx) = cx.add_window_view(|_, cx| Fixture::new(themes()[0], kind, false, cx));
        redraw(cx);
        let target = cx.debug_bounds("subject").unwrap().center();
        let outside = point(px(280.0), px(200.0));
        let other_button = if button == MouseButton::Left {
            MouseButton::Right
        } else {
            MouseButton::Left
        };

        for (from, to, release_button) in [
            (target, outside, button),
            (outside, target, button),
            (target, target, other_button),
        ] {
            cx.simulate_mouse_move(from, None, Modifiers::default());
            mouse_down(cx, from, button, true);
            redraw(cx);
            cx.simulate_mouse_move(to, Some(button), Modifiers::default());
            cx.simulate_mouse_up(to, release_button, Modifiers::default());
            cx.simulate_mouse_up(target, button, Modifiers::default());
            cx.update(|_, app| assert_eq!(view.read(app).clicks, 0, "{kind:?}: cancelled click"));
        }

        // Wayland and macOS mark the focusing press; the next press is ordinary.
        for (count, first_mouse) in [true, false].into_iter().enumerate() {
            cx.simulate_mouse_move(target, None, Modifiers::default());
            mouse_down(cx, target, button, first_mouse);
            redraw(cx);
            cx.update(|_, app| assert_eq!(view.read(app).clicks, count, "{kind:?}: press"));
            cx.simulate_mouse_up(target, button, Modifiers::default());
            cx.update(|_, app| {
                assert_eq!(
                    view.read(app).clicks,
                    count + 1,
                    "{kind:?}: completed click"
                );
            });
            cx.simulate_mouse_up(target, button, Modifiers::default());
            cx.update(|_, app| {
                assert_eq!(
                    view.read(app).clicks,
                    count + 1,
                    "{kind:?}: duplicate release"
                );
            });
        }
    }
}

#[test]
fn focusing_click_activates_text_subtarget_once() {
    let mut click = crate::kit::click::SubtargetClick::default();
    let up = gpui::MouseUpEvent {
        button: MouseButton::Left,
        click_count: 1,
        ..Default::default()
    };
    for first_mouse in [true, false] {
        let down = MouseDownEvent {
            button: MouseButton::Left,
            click_count: 1,
            first_mouse,
            ..Default::default()
        };
        click.press(Some("link"), &down);
        let Some(gpui::ClickEvent::Mouse(event)) = click.release(Some(&"link"), &up) else {
            panic!("a completed text-link click must activate, including the focusing click");
        };
        assert_eq!(event.down.first_mouse, first_mouse);
        assert!(click.release(Some(&"link"), &up).is_none());

        click.press(Some("link"), &down);
        assert!(click.release(Some(&"another link"), &up).is_none());
        assert!(click.release(Some(&"link"), &up).is_none());

        click.press(Some("link"), &down);
        assert!(
            click
                .release(
                    Some(&"link"),
                    &gpui::MouseUpEvent {
                        button: MouseButton::Right,
                        ..up.clone()
                    },
                )
                .is_none()
        );
        assert!(click.release(Some(&"link"), &up).is_none());

        click.press(Some("link"), &down);
        click.moved(&gpui::MouseMoveEvent {
            position: point(px(10.0), px(0.0)),
            pressed_button: Some(MouseButton::Left),
            modifiers: Modifiers::default(),
        });
        assert!(click.release(Some(&"link"), &up).is_none());
    }
}

#[gpui::test]
fn mouse_focus_does_not_latch_control_backgrounds(cx: &mut TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    for theme in themes() {
        for kind in [
            FixtureKind::Button(ButtonStyle::Filled),
            FixtureKind::Button(ButtonStyle::Outlined),
            FixtureKind::Button(ButtonStyle::Solid),
            FixtureKind::Button(ButtonStyle::Subtle),
            FixtureKind::Button(ButtonStyle::Transparent),
            FixtureKind::Button(ButtonStyle::Danger),
            FixtureKind::BoundedButton,
            FixtureKind::Row,
            FixtureKind::PanelTab,
            FixtureKind::Split,
        ] {
            let (view, cx) = cx.add_window_view(|_, cx| Fixture::new(theme, kind, true, cx));
            leave(cx);
            let resting = paint(cx, "subject");
            let bounds = cx.debug_bounds("subject").unwrap();
            cx.simulate_click(bounds.center(), Modifiers::default());
            leave(cx);
            cx.update(|window, app| assert!(view.read(app).focus.is_focused(window)));
            assert_eq!(
                paint(cx, "subject"),
                resting,
                "mouse focus must not paint a persistent fill"
            );
            // Escape changes input modality without changing the button's
            // semantic state. Keyboard focus must also leave its fill alone.
            cx.simulate_keystrokes("escape");
            redraw(cx);
            assert_eq!(
                paint(cx, "subject"),
                resting,
                "keyboard focus must not replace the fill"
            );
            cx.simulate_mouse_down(bounds.center(), MouseButton::Left, Modifiers::default());
            cx.simulate_mouse_move(
                point(px(280.0), px(200.0)),
                Some(MouseButton::Left),
                Modifiers::default(),
            );
            cx.simulate_mouse_up(
                point(px(280.0), px(200.0)),
                MouseButton::Left,
                Modifiers::default(),
            );
            redraw(cx);
            assert_eq!(
                paint(cx, "subject"),
                resting,
                "cancelled press must release its styling"
            );
        }
    }
}

#[gpui::test]
fn disabled_buttons_share_activation_and_tab_policy(cx: &mut TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    for explicit in [false, true] {
        for kind in [
            FixtureKind::Button(ButtonStyle::Subtle),
            FixtureKind::BoundedButton,
        ] {
            let (view, cx) =
                cx.add_window_view(|_, cx| Fixture::new(themes()[0], kind, explicit, cx));
            cx.update(|_, app| {
                view.update(app, |view, cx| {
                    view.state = view.state.disabled(true);
                    cx.notify();
                })
            });
            leave(cx);
            let resting = paint(cx, "subject");
            let bounds = cx.debug_bounds("subject").unwrap();
            cx.simulate_click(bounds.center(), Modifiers::default());
            mouse_down(cx, bounds.center(), MouseButton::Left, true);
            cx.simulate_mouse_up(bounds.center(), MouseButton::Left, Modifiers::default());
            leave(cx);
            assert_eq!(paint(cx, "subject"), resting);
            cx.update(|window, app| {
                assert_eq!(view.read(app).clicks, 0);
                let before = view.read(app).before.clone();
                window.focus(&before, app);
                window.focus_next(app);
                assert!(
                    view.read(app).after.is_focused(window),
                    "disabled subject must be skipped by Tab"
                );
            });
        }
    }
}

#[gpui::test]
fn split_segments_track_open_and_busy_independently(cx: &mut TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    for theme in themes() {
        let (view, cx) =
            cx.add_window_view(|_, cx| Fixture::new(theme, FixtureKind::Split, true, cx));
        leave(cx);
        let main_rest = paint(cx, "subject");
        let menu_rest = paint(cx, "menu");
        let menu = cx.debug_bounds("menu").unwrap();
        cx.simulate_click(menu.center(), Modifiers::default());
        leave(cx);
        assert_ne!(paint(cx, "menu"), menu_rest);
        assert_eq!(paint(cx, "subject"), main_rest);
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.menu_open = false;
                cx.notify();
            })
        });
        leave(cx);
        assert_eq!(paint(cx, "menu"), menu_rest);
        for in_flight in [2, 1, 0] {
            cx.update(|_, app| {
                view.update(app, |view, cx| {
                    view.state = view.state.busy(in_flight > 0);
                    cx.notify();
                })
            });
            redraw(cx);
            assert_eq!(paint(cx, "subject") == main_rest, in_flight == 0);
            assert_eq!(paint(cx, "menu"), menu_rest);
        }
        cx.update(|_, app| {
            assert_eq!(view.read(app).clicks, 0);
            assert_eq!(view.read(app).menu_clicks, 1);
        });
    }
}

#[test]
fn semantic_state_can_be_cleared_without_losing_selection_under_an_open_menu() {
    let theme = themes()[0];
    let style = InteractionStyle::accent(theme);
    let selected = gpui::rgb(0x123456);
    let state = InteractionState::default()
        .selected(true, selected)
        .open(true);
    assert_eq!(
        style.persistent_fill(state),
        Some(control_open_background(theme))
    );
    assert_eq!(style.persistent_fill(state.open(false)), Some(selected));
    assert_eq!(
        style.persistent_fill(
            state
                .open(false)
                .selected(false, selected)
                .busy(true)
                .busy(false)
        ),
        None
    );
}

#[gpui::test]
fn selected_controls_keep_their_fill_through_pointer_and_keyboard_focus(cx: &mut TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    for theme in themes() {
        for kind in [
            FixtureKind::Button(ButtonStyle::Subtle),
            FixtureKind::Row,
            FixtureKind::PanelTab,
        ] {
            let (view, cx) = cx.add_window_view(|_, cx| Fixture::new(theme, kind, true, cx));
            leave(cx);
            let resting = paint(cx, "subject");
            cx.update(|_, app| {
                view.update(app, |view, cx| {
                    view.state = view
                        .state
                        .selected(true, theme.colors.interaction.selected_background);
                    cx.notify();
                })
            });
            redraw(cx);
            let selected = paint(cx, "subject");
            assert_ne!(selected, resting);
            let bounds = cx.debug_bounds("subject").unwrap();
            cx.simulate_click(
                point(bounds.left() + px(4.0), bounds.center().y),
                Modifiers::default(),
            );
            redraw(cx);
            assert_eq!(paint(cx, "subject"), selected);
            leave(cx);
            cx.simulate_keystrokes("escape");
            redraw(cx);
            assert_eq!(paint(cx, "subject"), selected);
            cx.update(|_, app| {
                view.update(app, |view, cx| {
                    view.state = InteractionState::default();
                    cx.notify();
                })
            });
            redraw(cx);
            assert_eq!(paint(cx, "subject"), resting);
        }
    }
}

#[gpui::test]
fn nested_close_consumes_its_press_without_selecting_the_tab(cx: &mut TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (view, cx) =
        cx.add_window_view(|_, cx| Fixture::new(themes()[0], FixtureKind::PanelTab, true, cx));
    redraw(cx);
    let position = cx.debug_bounds("close").unwrap().center();
    cx.simulate_click(position, Modifiers::default());
    cx.update(|_, app| {
        assert_eq!(view.read(app).menu_clicks, 1);
        assert_eq!(
            view.read(app).clicks,
            0,
            "closing a tab must not select it too"
        );
    });
}

#[gpui::test]
fn nested_actions_cancel_on_release_outside_and_support_keyboard(cx: &mut TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (view, cx) =
        cx.add_window_view(|_, cx| Fixture::new(themes()[0], FixtureKind::PanelTab, false, cx));
    redraw(cx);
    let position = cx.debug_bounds("close").unwrap().center();
    cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
    cx.update(|_, app| assert_eq!(view.read(app).menu_clicks, 0));
    let outside = point(px(280.0), px(200.0));
    cx.simulate_mouse_move(outside, Some(MouseButton::Left), Modifiers::default());
    cx.simulate_mouse_up(outside, MouseButton::Left, Modifiers::default());
    redraw(cx);
    cx.update(|_, app| {
        assert_eq!(view.read(app).clicks, 0);
        assert_eq!(view.read(app).menu_clicks, 0);
    });
    // Nested pointer actions preserve the surrounding editor/list focus.
    // Keyboard navigation can explicitly reach the close action inside a tab.
    cx.update(|window, app| {
        let focus = view.read(app).focus.clone();
        window.focus(&focus, app);
        window.focus_next(app);
    });
    redraw(cx);
    cx.simulate_keystrokes("space");
    cx.update(|window, app| {
        window.dispatch_event(
            gpui::PlatformInput::KeyUp(gpui::KeyUpEvent {
                keystroke: gpui::Keystroke::parse("space").unwrap(),
            }),
            app,
        );
    });
    redraw(cx);
    cx.update(|_, app| {
        assert_eq!(view.read(app).menu_clicks, 1);
        assert_eq!(view.read(app).clicks, 0);
    });
}

#[gpui::test]
fn focused_actions_consume_activation_keys_before_parent_shortcuts(cx: &mut TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    for kind in [
        FixtureKind::PanelTab,
        FixtureKind::Button(ButtonStyle::Subtle),
    ] {
        for key in ["space", "enter"] {
            let (view, cx) = cx.add_window_view(|_, cx| Fixture::new(themes()[0], kind, false, cx));
            redraw(cx);
            cx.update(|window, app| {
                let before = view.read(app).before.clone();
                window.focus(&before, app);
                window.focus_next(app);
            });
            redraw(cx);
            cx.simulate_keystrokes(key);
            cx.update(|_, app| {
                assert_eq!(
                    view.read(app).parent_keys,
                    0,
                    "activation key-down reached the parent"
                );
                assert_eq!(view.read(app).clicks + view.read(app).menu_clicks, 0);
            });
            cx.update(|window, app| {
                window.dispatch_event(
                    gpui::PlatformInput::KeyUp(gpui::KeyUpEvent {
                        keystroke: gpui::Keystroke::parse(key).unwrap(),
                    }),
                    app,
                );
            });
            cx.update(|_, app| {
                assert_eq!(
                    view.read(app).parent_keys,
                    0,
                    "activation key-up reached the parent"
                );
                assert_eq!(view.read(app).clicks + view.read(app).menu_clicks, 1);
            });
        }
    }
}

#[gpui::test]
fn menu_activation_requires_a_completed_click_on_the_same_entry(cx: &mut TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    for kind in [FixtureKind::Menu, FixtureKind::AppMenu] {
        let (view, cx) = cx.add_window_view(|_, cx| Fixture::new(themes()[0], kind, false, cx));
        redraw(cx);
        let target = cx.debug_bounds("subject").unwrap().center();
        for button in [MouseButton::Left, MouseButton::Right] {
            cx.simulate_mouse_down(point(px(280.0), px(200.0)), button, Modifiers::default());
            cx.simulate_mouse_move(target, Some(button), Modifiers::default());
            cx.simulate_mouse_up(target, button, Modifiers::default());
            cx.update(|_, app| assert_eq!(view.read(app).clicks, 0));
        }
        for button in [MouseButton::Left, MouseButton::Right] {
            cx.simulate_mouse_down(target, button, Modifiers::default());
            redraw(cx);
            cx.simulate_mouse_up(target, button, Modifiers::default());
        }
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                assert_eq!(view.clicks, 2);
                view.state = view.state.disabled(true);
                cx.notify();
            })
        });
        redraw(cx);
        cx.simulate_click(target, Modifiers::default());
        cx.simulate_mouse_up(target, MouseButton::Right, Modifiers::default());
        cx.update(|_, app| assert_eq!(view.read(app).clicks, 2));
    }
}

#[gpui::test]
fn canvas_decorations_follow_the_whole_row_through_hover_press_and_rerender(
    cx: &mut TestAppContext,
) {
    use palette::IntoColor;
    let _guard = crate::test_support::lock_visual_test();
    fn assert_matching_fill(cx: &mut VisualTestContext) {
        let row: gpui::Rgba = paint(cx, "subject")[0].0.as_solid().unwrap().into_color();
        let mask: gpui::Rgba = paint(cx, "painted_mask")[0]
            .0
            .as_solid()
            .unwrap()
            .into_color();
        for (a, b) in [
            (row.red, mask.red),
            (row.green, mask.green),
            (row.blue, mask.blue),
            (row.alpha, mask.alpha),
        ] {
            assert!(
                (a - b).abs() < 0.00001,
                "canvas must match the row: {row:?} vs {mask:?}"
            );
        }
    }
    for theme in themes() {
        let (view, cx) =
            cx.add_window_view(|_, cx| Fixture::new(theme, FixtureKind::PaintedRow, false, cx));
        for state in [
            InteractionState::default(),
            InteractionState::default().selected(true, gpui::rgb(0x345678)),
            InteractionState::default().open(true),
            InteractionState::default().disabled(true),
        ] {
            cx.update(|_, app| {
                view.update(app, |view, cx| {
                    view.state = state;
                    cx.notify();
                })
            });
            leave(cx);
            assert_matching_fill(cx);
            let bounds = cx.debug_bounds("subject").unwrap();
            // The pointer is over the label, outside the narrow painted mask.
            let label = point(bounds.right() - px(4.0), bounds.center().y);
            cx.simulate_mouse_move(label, None, Modifiers::default());
            redraw(cx);
            assert_matching_fill(cx);
            cx.simulate_mouse_down(label, MouseButton::Left, Modifiers::default());
            cx.update(|_, app| view.update(app, |_, cx| cx.notify()));
            redraw(cx);
            assert_matching_fill(cx);
            let outside = point(px(280.0), px(200.0));
            cx.simulate_mouse_move(outside, Some(MouseButton::Left), Modifiers::default());
            redraw(cx);
            assert_matching_fill(cx);
            cx.simulate_mouse_up(outside, MouseButton::Left, Modifiers::default());
            redraw(cx);
            assert_matching_fill(cx);
        }
    }
}

#[test]
fn discrete_control_styles_and_activation_are_owned_by_the_interaction_kit() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let pattern = regex::Regex::new(r"\.(?:hover|active)\(\s*(?:move\s+)?\|").unwrap();
    let raw_click =
        regex::Regex::new(r"\.on_click\(\s*(?:cx\.listener|on_click\b|(?:move\s+)?\|)").unwrap();
    let raw_release = regex::Regex::new(r"\.on_mouse_up\(").unwrap();
    let context_press =
        regex::Regex::new(r"\.on_mouse_down\(\s*(?:gpui::)?MouseButton::Right").unwrap();
    let mut pending = vec![root.clone()];
    let mut violations = Vec::new();
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy();
            if name == "tests" || name == "benchmarks" || name.contains("tests") {
                continue;
            }
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            if path.extension().is_none_or(|extension| extension != "rs") {
                continue;
            }
            let relative = path.strip_prefix(&root).unwrap();
            // Editing focus is intentionally different from a discrete action.
            if relative == std::path::Path::new("kit/interaction.rs")
                || relative == std::path::Path::new("kit/text_input/render.rs")
            {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            if pattern.is_match(&source) {
                violations.push(format!(
                    "{}: custom hover/press styling",
                    relative.display()
                ));
            }
            if raw_click.is_match(&source) {
                violations.push(format!("{}: raw click activation", relative.display()));
            }
            // These releases end continuous gestures. Text links observe a
            // shared SubtargetClick because their input owns pointer selection.
            // Compare paths as paths so Windows separators match the allowlist.
            let release_gesture = [
                "kit/text_input/render.rs",
                "view/components/commit_link_menu.rs",
                "view/gitcomet_view.rs",
                "view/chrome.rs",
                "view/settings_window/render.rs",
                "view/terminal_panel.rs",
                "view/terminal_panel/viewport.rs",
                "view/panels/layout.rs",
                "view/panels/repo_tabs_bar.rs",
                "view/panes/history/history_panel.rs",
                "view/rows/diff_text/build.rs",
                "view/panels/main/conflict_resolver_view.rs",
                "view/panels/main/diff.rs",
                "view/panels/main/diff_view.rs",
            ]
            .iter()
            .any(|allowed| relative == std::path::Path::new(allowed));
            if raw_release.is_match(&source) && !release_gesture {
                violations.push(format!(
                    "{}: release without the shared click gate",
                    relative.display()
                ));
            }
            if context_press.is_match(&source)
                && ![
                    "kit/text_input/editing.rs",
                    "kit/text_input/render.rs",
                    "view/terminal_panel/viewport.rs",
                ]
                .iter()
                .any(|allowed| relative == std::path::Path::new(allowed))
            {
                violations.push(format!("{}: context action on press", relative.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "Use the shared interaction APIs for discrete controls: {violations:?}"
    );
}

#[gpui::test]
fn controls_never_transfer_clicks_to_another_element(cx: &mut TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    for kind in [
        FixtureKind::Button(ButtonStyle::Subtle),
        FixtureKind::BoundedButton,
        FixtureKind::Row,
        FixtureKind::Menu,
        FixtureKind::AppMenu,
        FixtureKind::Split,
    ] {
        let (view, cx) = cx.add_window_view(|_, cx| Fixture::new(themes()[0], kind, false, cx));
        redraw(cx);
        let a = cx.debug_bounds("before").unwrap().center();
        let b = cx.debug_bounds("subject").unwrap().center();
        let outside = point(px(280.0), px(200.0));
        for (from, to) in [(a, b), (b, a), (b, outside), (outside, b)] {
            cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
            redraw(cx);
            cx.simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::default());
            cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
            redraw(cx);
            cx.update(|_, app| {
                assert_eq!(
                    view.read(app).clicks
                        + view.read(app).source_clicks
                        + view.read(app).menu_clicks,
                    0
                )
            });
        }
        cx.simulate_mouse_down(b, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(b, MouseButton::Right, Modifiers::default());
        cx.simulate_mouse_up(b, MouseButton::Left, Modifiers::default());
        cx.update(|_, app| assert_eq!(view.read(app).clicks, 0, "mismatched buttons must cancel"));
        cx.simulate_click(b, Modifiers::default());
        cx.update(|_, app| {
            assert_eq!(
                view.read(app).clicks,
                1,
                "a completed click must still activate once"
            )
        });
    }
}

#[gpui::test]
fn split_button_segments_cannot_complete_each_others_press(cx: &mut TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (view, cx) =
        cx.add_window_view(|_, cx| Fixture::new(themes()[0], FixtureKind::Split, false, cx));
    redraw(cx);
    let action = cx.debug_bounds("subject").unwrap().center();
    let menu = cx.debug_bounds("menu").unwrap().center();
    for (from, to) in [(action, menu), (menu, action)] {
        cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::default());
        cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
    }
    cx.update(|_, app| {
        assert_eq!(view.read(app).clicks, 0);
        assert_eq!(view.read(app).menu_clicks, 0);
    });
}

#[gpui::test]
fn disabling_a_pressed_button_cancels_the_pending_action(cx: &mut TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (view, cx) = cx.add_window_view(|_, cx| {
        Fixture::new(
            themes()[0],
            FixtureKind::Button(ButtonStyle::Subtle),
            false,
            cx,
        )
    });
    redraw(cx);
    let position = cx.debug_bounds("subject").unwrap().center();
    cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
    cx.update(|_, app| {
        view.update(app, |this, cx| {
            this.state = this.state.disabled(true);
            cx.notify();
        })
    });
    redraw(cx);
    cx.simulate_mouse_up(position, MouseButton::Left, Modifiers::default());
    cx.update(|_, app| assert_eq!(view.read(app).clicks, 0));
}

#[gpui::test]
fn keyboard_activation_cancels_after_focus_changes_and_keeps_modified_shortcuts(
    cx: &mut TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (view, cx) = cx.add_window_view(|_, cx| {
        Fixture::new(
            themes()[0],
            FixtureKind::Button(ButtonStyle::Subtle),
            true,
            cx,
        )
    });
    cx.update(|window, app| {
        let focus = view.read(app).focus.clone();
        window.focus(&focus, app);
    });
    redraw(cx);
    cx.simulate_keystrokes("space");
    cx.update(|window, app| {
        let after = view.read(app).after.clone();
        let focus = view.read(app).focus.clone();
        window.focus(&after, app);
        window.focus(&focus, app);
    });
    redraw(cx);
    cx.update(|window, app| {
        window.dispatch_event(
            gpui::PlatformInput::KeyUp(gpui::KeyUpEvent {
                keystroke: gpui::Keystroke::parse("space").unwrap(),
            }),
            app,
        )
    });
    cx.update(|_, app| assert_eq!(view.read(app).clicks, 0));
    cx.simulate_keystrokes("ctrl-enter");
    cx.update(|window, app| {
        window.dispatch_event(
            gpui::PlatformInput::KeyUp(gpui::KeyUpEvent {
                keystroke: gpui::Keystroke::parse("ctrl-enter").unwrap(),
            }),
            app,
        )
    });
    cx.update(|_, app| {
        assert_eq!(view.read(app).clicks, 0);
        assert_eq!(
            view.read(app).parent_keys,
            2,
            "modified activation keys retain their shortcut routing"
        );
    });
}

#[gpui::test]
fn repeated_local_ids_do_not_transfer_nested_click_ownership(cx: &mut TestAppContext) {
    struct NestedFixture {
        parent_clicks: usize,
        child_clicks: usize,
    }
    impl Render for NestedFixture {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .id("same_id")
                .debug_selector(|| "parent".into())
                .size(px(160.0))
                .p(px(10.0))
                .on_activate(
                    false,
                    ControlActivation::Composite,
                    cx.listener(|this, _, _, _| this.parent_clicks += 1),
                )
                .child(
                    div()
                        .id("same_id")
                        .debug_selector(|| "child".into())
                        .size(px(40.0))
                        .on_activate(
                            false,
                            ControlActivation::Action,
                            cx.listener(|this, _, _, _| this.child_clicks += 1),
                        ),
                )
        }
    }
    let _guard = crate::test_support::lock_visual_test();
    let (view, cx) = cx.add_window_view(|_, _| NestedFixture {
        parent_clicks: 0,
        child_clicks: 0,
    });
    redraw(cx);
    let child = cx.debug_bounds("child").unwrap().center();
    let parent = cx.debug_bounds("parent").unwrap().center();
    for (from, to) in [(child, parent), (parent, child)] {
        cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
        cx.update(|_, app| view.update(app, |_, cx| cx.notify()));
        redraw(cx);
        cx.simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::default());
        cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
        cx.update(|_, app| {
            assert_eq!(view.read(app).parent_clicks, 0);
            assert_eq!(view.read(app).child_clicks, 0);
        });
    }
    for at in [child, parent] {
        cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
    }
    cx.update(|_, app| {
        assert_eq!(view.read(app).parent_clicks, 1);
        assert_eq!(view.read(app).child_clicks, 1);
    });
}

#[gpui::test]
fn idle_pointer_moves_leave_the_press_state_alone(cx: &mut TestAppContext) {
    // Clearing the shared press state writes a new global and queues an
    // observer effect; with no press pending there is nothing to clear.
    let _guard = crate::test_support::lock_visual_test();
    let (view, cx) =
        cx.add_window_view(|_, cx| Fixture::new(themes()[0], FixtureKind::Row, false, cx));
    redraw(cx);
    let subject = cx.debug_bounds("subject").unwrap();

    crate::kit::click::take_click_state_writes_for_tests();
    for step in 0..5 {
        cx.simulate_mouse_move(
            point(subject.left() + px(2.0 + step as f32), subject.center().y),
            None,
            Modifiers::default(),
        );
    }
    assert_eq!(
        crate::kit::click::take_click_state_writes_for_tests(),
        0,
        "moving over a control with nothing pressed writes no press state"
    );

    cx.simulate_click(subject.center(), Modifiers::default());
    cx.update(|_, app| assert_eq!(view.read(app).clicks, 1, "a click still completes"));
}
