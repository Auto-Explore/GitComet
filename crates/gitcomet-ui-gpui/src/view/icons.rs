use gpui::{Animation, AnimationExt, ElementId, IntoElement, Pixels, Styled, Transformation};

pub(in crate::view) const STASH_ICON_PATH: &str = "icons/stash.svg";
pub(in crate::view) const GIT_MERGE_ICON_PATH: &str = "icons/git_merge.svg";
/// Graph-node variant of [`STASH_ICON_PATH`]: same artwork with a heavier stroke
/// so it survives being knocked out of a 16px node. The retained-mode icon keeps
/// its own weight for the sidebar and action bar.
pub(in crate::view) const GIT_STASH_NODE_ICON_PATH: &str = "icons/git_stash.svg";
/// Marks the "Uncommitted changes" nodes. Lucide `code` — two chevrons, which
/// is about the most detail that survives being knocked out of a 16px node.
/// Node-only, hence the heavier stroke than the retained-mode icons.
pub(in crate::view) const UNCOMMITTED_NODE_ICON_PATH: &str = "icons/code.svg";

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
