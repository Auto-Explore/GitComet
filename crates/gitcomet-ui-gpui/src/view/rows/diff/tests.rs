use super::*;

/// Wrap columns come from a measured character, so that measurement has to
/// answer the editor font size and the UI zoom. (Family cannot be checked
/// here: the test text system measures every font identically.)
#[gpui::test]
fn wrap_char_width_follows_the_editor_font_and_zoom(cx: &mut gpui::TestAppContext) {
    struct Probe;
    impl gpui::Render for Probe {
        fn render(
            &mut self,
            _window: &mut Window,
            _cx: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            gpui::div()
        }
    }

    let (_view, cx) = cx.add_window_view(|_window, _cx| Probe);
    cx.update(|window, _app| {
        fn width(window: &mut Window, size: u32) -> Pixels {
            crate::view::rows::diff_canvas_text_wrap_char_width(
                window,
                crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY.to_string(),
                size,
            )
        }
        fn columns(char_width: Pixels) -> usize {
            (f32::from(px(400.0)) / f32::from(char_width)).floor() as usize
        }

        let (small, default, large) = (width(window, 8), width(window, 13), width(window, 26));
        assert!(small < default && default < large);
        assert!(
            columns(large) < columns(default),
            "a larger font must wrap sooner"
        );

        crate::ui_scale::apply_to_window(window, 200);
        let zoomed = width(window, 13);
        crate::ui_scale::apply_to_window(window, 100);
        assert!(zoomed > default, "zoom must widen the measured character");
    });
}
