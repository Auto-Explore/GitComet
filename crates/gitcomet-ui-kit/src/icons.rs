use gpui::{Animation, AnimationExt, ElementId, IntoElement, Pixels, Styled, Transformation};

pub const STASH_ICON_PATH: &str = "icons/stash.svg";
pub const GIT_MERGE_ICON_PATH: &str = "icons/git_merge.svg";
/// Graph-node variant of [`STASH_ICON_PATH`]: same artwork with a heavier stroke
/// so it survives being knocked out of a 16px node. The retained-mode icon keeps
/// its own weight for the sidebar and action bar.
pub const GIT_STASH_NODE_ICON_PATH: &str = "icons/git_stash.svg";
/// Marks the "Uncommitted changes" nodes. Lucide `code` — two chevrons, which
/// is about the most detail that survives being knocked out of a 16px node.
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

pub fn svg_spinner(id: impl Into<ElementId>, color: gpui::Rgba, size: Pixels) -> impl IntoElement {
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

#[cfg(test)]
mod tests {
    use crate::test_support::source_guards::{icon_size_args, unscaled_icon_sizes};

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
