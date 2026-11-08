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
