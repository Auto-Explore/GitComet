use gpui::{Animation, AnimationExt, ElementId, IntoElement, Pixels, Styled, Transformation};

pub(in crate::view) const STASH_ICON_PATH: &str = "icons/stash.svg";

pub(super) fn svg_icon(path: &'static str, color: gpui::Rgba, size: Pixels) -> gpui::Svg {
    gpui::svg()
        .path(path)
        .w(size)
        .h(size)
        .text_color(color)
        .flex_shrink_0()
}

pub(super) fn svg_spinner(
    id: impl Into<ElementId>,
    color: gpui::Rgba,
    size: Pixels,
) -> impl IntoElement {
    gpui::svg()
        .path("icons/spinner.svg")
        .w(size)
        .h(size)
        .text_color(color)
        .flex_shrink_0()
        .with_animation(
            id,
            Animation::new(std::time::Duration::from_millis(850)).repeat(),
            |svg, delta| {
                svg.with_transformation(Transformation::rotate(gpui::radians(
                    delta * std::f32::consts::TAU,
                )))
            },
        )
}
