use super::*;

struct ScrollableField {
    input: Entity<TextInput>,
    scroll: ScrollHandle,
}

impl Render for ScrollableField {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(div().w(px(280.0)).child(self.input.clone()))
    }
}

fn fixture(
    cx: &mut gpui::TestAppContext,
) -> (Entity<ScrollableField>, &mut gpui::VisualTestContext) {
    crate::test_support::use_real_text_backend(cx);
    cx.add_window_view(|window, cx| {
        window.activate();
        let scroll = ScrollHandle::new();
        let input = cx.new(|cx| {
            let mut input = TextInput::new(
                TextInputOptions {
                    multiline: true,
                    soft_wrap: true,
                    ..Default::default()
                },
                window,
                cx,
            );
            input.set_scrollable(scroll.clone(), 140.0, cx);
            window.focus(&input.focus_handle(), cx);
            input
        });
        ScrollableField { input, scroll }
    })
}

fn settle(cx: &mut gpui::VisualTestContext) {
    for _ in 0..4 {
        crate::test_support::refresh_and_draw(cx);
    }
}

#[gpui::test]
fn internal_scrolling_keeps_the_border_full_width_and_wrapping_stable(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let (input, scroll) = cx.update(|_, app| {
        let view = view.read(app);
        (view.input.clone(), view.scroll.clone())
    });
    for percent in [100, 150] {
        cx.update(|_, app| {
            crate::ui_scale::set_default(app, percent);
            input.update(app, |input, cx| input.set_text("alpha beta gamma", cx));
        });
        settle(cx);
        let short_field = cx.debug_bounds("scrollable_text_input_field").unwrap();
        assert_eq!(short_field.size.width, px(280.0));
        assert_eq!(scroll.max_offset().y, px(0.0));
        let text_width = cx.update(|_, app| input.read(app).wrap.cache.unwrap().width);
        cx.update(|_, app| {
            input.update(app, |input, cx| {
                input.set_text("alpha beta gamma delta epsilon ".repeat(80), cx);
            })
        });
        settle(cx);
        let field = cx.debug_bounds("scrollable_text_input_field").unwrap();
        assert_eq!(field.size.width, short_field.size.width);
        assert!(field.size.height > short_field.size.height);
        assert!(field.size.height <= px(140.0 * percent as f32 / 100.0) + px(1.0));
        assert!(scroll.max_offset().y > px(0.0));
        let viewport = scroll.bounds();
        assert!(viewport.left() >= field.left() && viewport.right() <= field.right());
        assert!(viewport.top() >= field.top() && viewport.bottom() <= field.bottom());
        cx.update(|window, app| {
            let input = input.read(app);
            assert_eq!(
                input.wrap.cache.unwrap().width,
                text_width,
                "overflow must not change wrapping width"
            );
            let scale = window.scale_factor();
            assert!(
                window.painted_quads().iter().any(|quad| {
                    quad.border_color == input.style.focus_border.into()
                        && (quad.bounds.size.width.0 - f32::from(field.size.width) * scale).abs()
                            < 1.0
                        && (quad.bounds.size.height.0 - f32::from(field.size.height) * scale).abs()
                            < 1.0
                }),
                "the focus outline must enclose the whole field, including the scrollbar"
            );
        });
    }
}

#[gpui::test]
fn internal_scrolling_reveals_the_caret_and_shrinks_back_after_clearing(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let (input, scroll) = cx.update(|_, app| {
        let view = view.read(app);
        (view.input.clone(), view.scroll.clone())
    });
    cx.update(|_, app| {
        input.update(app, |input, cx| {
            input.set_text(
                (0..50)
                    .map(|ix| format!("message line {ix}\n"))
                    .collect::<String>(),
                cx,
            );
        })
    });
    settle(cx);
    let tall_height = scroll.bounds().size.height;
    cx.update(|_, app| {
        input.update(app, |input, cx| {
            input.set_caret(input.text().len(), cx);
            input.ensure_cursor_visible_in_vertical_scroll(cx);
        })
    });
    settle(cx);
    assert!(
        scroll.offset().y < px(0.0),
        "the caret must scroll into view"
    );
    cx.update(|_, app| input.update(app, |input, cx| input.set_text("", cx)));
    settle(cx);
    assert_eq!(scroll.max_offset().y, px(0.0));
    assert_eq!(scroll.offset().y, px(0.0));
    assert!(scroll.bounds().size.height < tall_height);
}

#[cfg(target_os = "macos")]
fn elastic_draw(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, app| {
        window.simulate_next_frame(app);
        let _ = window.draw(app);
    });
    cx.run_until_parked();
}

#[cfg(target_os = "macos")]
fn elastic_wheel(
    cx: &mut gpui::VisualTestContext,
    position: Point<Pixels>,
    delta: f32,
    phase: gpui::TouchPhase,
) {
    cx.simulate_event(gpui::ScrollWheelEvent {
        position,
        delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(delta))),
        touch_phase: phase,
        ..Default::default()
    });
    elastic_draw(cx);
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn internally_scrolling_input_stretches_text_with_caret_and_ime_inside_fixed_chrome(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let (input, scroll) = cx.update(|_, app| {
        let view = view.read(app);
        (view.input.clone(), view.scroll.clone())
    });
    cx.update(|_, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha beta gamma\n".repeat(50), cx);
            input.set_caret(0, cx);
        });
    });
    settle(cx);
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        cx.update(|window, _| {
            window.activate();
            window.refresh();
        });
        elastic_draw(cx);
        let field = cx.debug_bounds("scrollable_text_input_field").unwrap();
        let before = cx.update(|window, app| {
            input.update(app, |input, cx| {
                let bounds = input.layout.bounds.unwrap();
                let ime = input.bounds_for_range(0..0, bounds, window, cx).unwrap();
                let thumb = window
                    .painted_quads()
                    .into_iter()
                    .find(|quad| {
                        quad.background == input.style.menu_theme.colors.scrollbar.thumb.into()
                    })
                    .expect("an overflowing field paints its scrollbar")
                    .bounds;
                (bounds, ime, thumb)
            })
        });
        for (delta, phase) in [
            (100.0, gpui::TouchPhase::Started),
            (0.0, gpui::TouchPhase::Ended),
        ] {
            elastic_wheel(cx, field.center(), delta, phase);
            assert_eq!(
                cx.debug_bounds("scrollable_text_input_field").unwrap(),
                field
            );
            assert_eq!(scroll.offset().y, px(0.0));
            cx.update(|window, app| {
                input.update(app, |input, cx| {
                    let bounds = input.layout.bounds.unwrap();
                    let shift = bounds.top() - before.0.top();
                    assert!(shift > px(20.0));
                    let ime = input.bounds_for_range(0..0, bounds, window, cx).unwrap();
                    assert_eq!(ime.top() - before.1.top(), shift);
                    let pointer = point(ime.left() + px(0.25), ime.top() + px(2.0));
                    assert_eq!(
                        input.character_index_for_point(pointer, window, cx),
                        Some(0)
                    );
                    assert_eq!(input.selected_range(), 0..0);
                    let thumb = window
                        .painted_quads()
                        .into_iter()
                        .find(|quad| {
                            quad.background == input.style.menu_theme.colors.scrollbar.thumb.into()
                        })
                        .unwrap()
                        .bounds;
                    assert_eq!(
                        thumb, before.2,
                        "the scrollbar stays in the fixed field frame"
                    );
                });
            });
        }

        // A caret reveal wins over an unfinished rebound. The new layout and
        // platform IME rectangle must agree at the document's bottom.
        cx.update(|window, app| {
            input.update(app, |input, cx| {
                let end = input.text().len();
                input.set_selected_range(end..end, true, window, cx);
            });
        });
        for _ in 0..4 {
            elastic_draw(cx);
        }
        assert!(scroll.offset().y < px(0.0));
        cx.update(|window, app| {
            input.update(app, |input, cx| {
                assert_eq!(
                    input.interaction.internal_elastic_scroll.displacement(),
                    px(0.0)
                );
                let bounds = input.layout.bounds.unwrap();
                let end = input.text().encode_utf16().count();
                let ime = input
                    .bounds_for_range(end..end, bounds, window, cx)
                    .unwrap();
                assert!(ime.top() >= field.top() && ime.bottom() <= field.bottom());
            });
        });
        let bottom = cx.update(|_, app| input.read(app).layout.bounds.unwrap());
        let offset = scroll.offset();
        elastic_wheel(cx, field.center(), -100.0, gpui::TouchPhase::Started);
        assert_eq!(scroll.offset(), offset);
        assert!(
            cx.update(|_, app| input.read(app).layout.bounds.unwrap().top())
                < bottom.top() - px(20.0)
        );
        // Composing text clears stretch without altering the IME's UTF-16 span.
        cx.update(|window, app| {
            input.update(app, |input, cx| {
                let end = input.text().encode_utf16().count();
                input.replace_and_mark_text_in_range(None, "猫", Some(1..1), window, cx);
                assert_eq!(input.marked_text_range(window, cx), Some(end..end + 1));
            });
        });
        for _ in 0..4 {
            elastic_draw(cx);
        }
        cx.update(|window, app| {
            input.update(app, |input, cx| {
                assert_eq!(
                    input.interaction.internal_elastic_scroll.displacement(),
                    px(0.0)
                );
                let bounds = input.layout.bounds.unwrap();
                let end = input.text().encode_utf16().count();
                let ime = input
                    .bounds_for_range(end..end, bounds, window, cx)
                    .unwrap();
                assert!(ime.top() >= field.top() && ime.bottom() <= field.bottom());
            });
        });
    });
}

#[cfg(target_os = "macos")]
struct ExternalElasticField {
    input: Entity<TextInput>,
    scroll: ScrollHandle,
    spring: crate::elastic_scroll::ElasticScrollState,
}

#[cfg(target_os = "macos")]
impl Render for ExternalElasticField {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(280.0)).h(px(140.0)).child(
            crate::components::ScrollContainer::vertical(
                "external_input_scroll",
                "external_input_scrollbar",
                self.scroll.clone(),
                px(140.0),
            )
            .motion_state(self.spring.clone())
            .render(AppTheme::gitcomet_dark(), self.input.clone()),
        )
    }
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn externally_owned_editor_spring_clears_on_caret_reveal_without_an_internal_wrapper(
    cx: &mut gpui::TestAppContext,
) {
    crate::test_support::use_real_text_backend(cx);
    let (view, cx) = cx.add_window_view(|window, cx| {
        window.activate();
        let scroll = ScrollHandle::new();
        let spring = crate::elastic_scroll::ElasticScrollState::default();
        let input = cx.new(|cx| {
            let mut input = TextInput::new(
                TextInputOptions {
                    multiline: true,
                    chromeless: true,
                    ..Default::default()
                },
                window,
                cx,
            );
            input.set_vertical_scroll_handle(Some(scroll.clone()));
            input.set_elastic_scroll_states(Some(spring.clone()), None);
            input.set_text("row\n".repeat(50), cx);
            input.set_caret(0, cx);
            window.focus(&input.focus_handle(), cx);
            input
        });
        ExternalElasticField {
            input,
            scroll,
            spring,
        }
    });
    settle(cx);
    let (input, scroll, spring) = cx.update(|_, app| {
        let view = view.read(app);
        (view.input.clone(), view.scroll.clone(), view.spring.clone())
    });
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        cx.update(|window, _| {
            window.activate();
            window.refresh();
        });
        elastic_draw(cx);
        let top = cx.update(|_, app| input.read(app).layout.bounds.unwrap());
        elastic_wheel(
            cx,
            point(px(100.0), px(50.0)),
            100.0,
            gpui::TouchPhase::Started,
        );
        assert!(spring.displacement() > px(20.0));
        assert!(
            cx.update(|_, app| input.read(app).layout.bounds.unwrap().top()) > top.top() + px(20.0)
        );
        assert_eq!(scroll.offset().y, px(0.0));
        cx.update(|window, app| {
            input.update(app, |input, cx| {
                assert!(input.scroll_max_height.is_none());
                let end = input.text().len();
                input.set_selected_range(end..end, true, window, cx);
            });
        });
        for _ in 0..4 {
            elastic_draw(cx);
        }
        assert_eq!(spring.displacement(), px(0.0));
        assert!(scroll.offset().y < px(0.0));
    });
}
