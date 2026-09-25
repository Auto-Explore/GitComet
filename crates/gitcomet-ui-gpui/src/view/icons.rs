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

pub(crate) fn svg_icon(path: &'static str, color: gpui::Rgba, size: Pixels) -> gpui::Svg {
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

#[cfg(test)]
mod tests {
    /// The window chrome holds one size at every UI scale, so only it may pass
    /// raw `px()` icon sizes.
    const FIXED_SCALE_SOURCES: [&str; 2] = ["view/chrome.rs", "view/panels/repo_tabs_bar.rs"];

    /// The size argument of every `svg_icon(`/`svg_spinner(` call in `source`.
    fn icon_size_args(source: &str) -> Vec<(usize, String)> {
        let mut sizes = Vec::new();
        for call in ["svg_icon(", "svg_spinner("] {
            for (start, _) in source.match_indices(call) {
                let preceded_by_ident = source[..start]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_');
                if preceded_by_ident || source[..start].ends_with("fn ") {
                    continue;
                }
                let args_start = start + call.len();
                let (mut depth, mut arg_start, mut last_arg) = (0usize, args_start, "");
                for (offset, c) in source[args_start..].char_indices() {
                    let at = args_start + offset;
                    match c {
                        '(' | '[' | '{' => depth += 1,
                        ')' | ']' | '}' if depth > 0 => depth -= 1,
                        ',' | ')' if depth == 0 => {
                            // Skipping empty slots tolerates a trailing comma.
                            let arg = source[arg_start..at].trim();
                            if !arg.is_empty() {
                                last_arg = arg;
                            }
                            arg_start = at + 1;
                            if c == ')' {
                                let line = source[..start].lines().count();
                                sizes.push((line, last_arg.to_string()));
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        sizes
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
        let mut sources = Vec::new();
        crate::test_support::rust_sources_under(&src_dir, &mut sources);

        let mut unscaled = Vec::new();
        for path in sources {
            let relative = path.strip_prefix(&src_dir).expect("source below src");
            let relative_str = relative.to_string_lossy().replace('\\', "/");
            let is_test_source = relative
                .components()
                .any(|component| component.as_os_str() == "tests")
                || relative_str.ends_with("tests.rs")
                || relative_str.ends_with("test_support.rs");
            if is_test_source || FIXED_SCALE_SOURCES.contains(&relative_str.as_str()) {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("read Rust source");
            let production = source.split("#[cfg(test)]\nmod tests").next().unwrap_or("");
            for (line, arg) in icon_size_args(production) {
                if arg.starts_with("px(") {
                    unscaled.push(format!("{relative_str}:{line}: {arg}"));
                }
            }
        }
        assert!(
            unscaled.is_empty(),
            "icon sizes must go through the UI scale (`scaled_px`, `ui_scale.px`): {unscaled:#?}"
        );
    }
}
