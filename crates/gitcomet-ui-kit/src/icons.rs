use gpui::{Animation, AnimationExt, ElementId, IntoElement, Pixels, Styled, Transformation};

pub const STASH_ICON_PATH: &str = "icons/stash.svg";
pub const GIT_MERGE_ICON_PATH: &str = "icons/git_merge.svg";
/// Graph-node variant of [`STASH_ICON_PATH`]: same artwork with a heavier stroke
/// so it stays legible inside a 16px node. The retained-mode icon keeps
/// its own weight for the sidebar and action bar.
pub const GIT_STASH_NODE_ICON_PATH: &str = "icons/git_stash.svg";
/// Marks the "Uncommitted changes" nodes. Lucide `code` — two chevrons, which
/// stay legible inside a 16px node.
/// Node-only, hence the heavier stroke than the retained-mode icons.
pub const UNCOMMITTED_NODE_ICON_PATH: &str = "icons/code.svg";

pub fn svg_icon(path: impl Into<gpui::SharedString>, color: gpui::Rgba, size: Pixels) -> gpui::Svg {
    gpui::svg()
        .path(path)
        .w(size)
        .h(size)
        .text_color(color)
        .flex_shrink_0()
}

/// Spinner frame rate. Every animation frame redraws the whole window, so an
/// uncapped spinner costs a full frame per display refresh (60-120 Hz) for as
/// long as it is visible; at 30 Hz the 850 ms turn still reads as smooth.
const SPINNER_MAX_FPS: f32 = 30.0;

pub fn svg_spinner(id: impl Into<ElementId>, color: gpui::Rgba, size: Pixels) -> impl IntoElement {
    let spinner = svg_icon("icons/spinner.svg", color, size);
    if !crate::ui_runtime::current().uses_spinner_animation() {
        return spinner.into_any_element();
    }
    spinner
        .with_animation(
            id,
            Animation::new(std::time::Duration::from_millis(850))
                .repeat()
                .with_max_fps(SPINNER_MAX_FPS),
            |svg, delta| {
                svg.with_transformation(Transformation::rotate(gpui::radians(
                    delta * std::f32::consts::TAU,
                )))
            },
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::svg_spinner;
    use crate::test_support::source_guards::{icon_size_args, unscaled_icon_sizes};
    use crate::ui_runtime::{UiRuntime, with_override};
    use gpui::{Context, IntoElement, Render, Window, px, rgba};
    use std::time::Duration;

    struct SpinnerView {
        renders: usize,
    }

    impl Render for SpinnerView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            self.renders += 1;
            svg_spinner("spinner", rgba(0xffffffff), px(16.0))
        }
    }

    fn spinner_renders_during_one_second(cx: &mut gpui::TestAppContext) -> usize {
        let (view, cx) = cx.add_window_view(|_window, _cx| SpinnerView { renders: 0 });
        crate::test_support::redraw(cx);
        let before = cx.update(|_window, app| view.read(app).renders);
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        cx.update(|_window, app| view.read(app).renders) - before
    }

    #[gpui::test]
    fn deterministic_spinner_does_not_redraw_as_the_clock_advances(cx: &mut gpui::TestAppContext) {
        assert_eq!(spinner_renders_during_one_second(cx), 0);
    }

    #[gpui::test]
    fn live_spinner_redraws_as_the_clock_advances(cx: &mut gpui::TestAppContext) {
        let renders = with_override(UiRuntime::live(), || spinner_renders_during_one_second(cx));
        assert!(
            renders >= 10,
            "a live spinner animates; got {renders} renders"
        );
    }

    #[test]
    fn icon_size_args_finds_the_last_argument() {
        let source = "svg_icon(p, c, px(12.0))\nsvg_spinner(id, c,\n    scaled_px(f(1, 2)),\n)";
        let sizes: Vec<_> = icon_size_args(source)
            .into_iter()
            .map(|(_, arg)| arg)
            .collect();
        assert_eq!(sizes, ["px(12.0)", "scaled_px(f(1, 2))"]);
    }

    /// A raw `px()` icon keeps its 100% size while the UI around it zooms.
    #[test]
    fn icon_sizes_follow_the_ui_scale() {
        let src_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let unscaled = unscaled_icon_sizes(&src_dir, &[]);
        assert!(
            unscaled.is_empty(),
            "icon sizes must go through the UI scale (`scaled_px`, `ui_scale.px`): {unscaled:#?}"
        );
    }
}
