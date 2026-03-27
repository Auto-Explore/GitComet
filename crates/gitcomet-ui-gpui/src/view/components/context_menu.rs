use crate::theme::AppTheme;
use gpui::prelude::*;
use gpui::{CursorStyle, Div, ElementId, SharedString, Stateful, div, px};

use super::CONTROL_HEIGHT_MD_PX;

pub fn context_menu(theme: AppTheme, content: impl IntoElement) -> Div {
    div()
        .w_full()
        .min_w_full()
        .flex()
        .flex_col()
        .text_color(theme.colors.text)
        .child(content)
}

pub fn context_menu_header(theme: AppTheme, title: impl Into<SharedString>) -> Div {
    div()
        .w_full()
        .px_2()
        .py_1()
        .text_xs()
        .line_clamp(1)
        .whitespace_nowrap()
        .overflow_hidden()
        .text_color(theme.colors.text_muted)
        .child(title.into())
}

pub fn context_menu_label(theme: AppTheme, text: impl Into<SharedString>) -> Div {
    div()
        .w_full()
        .px_2()
        .pb_1()
        .text_sm()
        .text_color(theme.colors.text)
        .line_clamp(2)
        .child(text.into())
}

pub fn context_menu_separator(theme: AppTheme) -> Div {
    div()
        .w_full()
        .border_t_1()
        .border_color(theme.colors.border)
}

pub fn context_menu_entry(
    id: impl Into<ElementId>,
    theme: AppTheme,
    selected: bool,
    disabled: bool,
    icon: Option<SharedString>,
    label: impl Into<SharedString>,
    shortcut: Option<SharedString>,
) -> Stateful<Div> {
    let label: SharedString = label.into();
    let icon_path = icon
        .as_ref()
        .and_then(|icon| context_menu_icon_path(icon.as_ref(), label.as_ref()));
    let icon_color = context_menu_icon_color(theme, disabled, label.as_ref(), icon_path);

    let mut row = div()
        .id(id)
        .h(px(CONTROL_HEIGHT_MD_PX))
        .w_full()
        .min_w_full()
        .px_2()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .rounded(px(theme.radii.row))
        .text_color(theme.colors.text)
        .when(selected, |s| s.bg(theme.colors.hover))
        .when(!disabled, |s| {
            s.cursor(CursorStyle::PointingHand)
                .hover(move |s| s.bg(theme.colors.hover))
                .active(move |s| s.bg(theme.colors.active))
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .flex_1()
                .min_w(px(0.0))
                .child(
                    div()
                        .w(px(16.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .when_some(icon_path, |this, path| {
                            this.child(crate::view::icons::svg_icon(path, icon_color, px(13.0)))
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_sm()
                        .line_clamp(1)
                        .child(label),
                ),
        );

    let mut end = div()
        .flex()
        .items_center()
        .gap_2()
        .font_family("monospace")
        .text_xs()
        .text_color(theme.colors.text_muted);

    if let Some(shortcut) = shortcut {
        end = end.child(shortcut);
    }
    row = row.child(end);

    if disabled {
        row = row
            .text_color(theme.colors.text_muted)
            .cursor(CursorStyle::Arrow);
    }

    row
}

fn context_menu_icon_color(
    theme: AppTheme,
    disabled: bool,
    label: &str,
    icon_path: Option<&'static str>,
) -> gpui::Rgba {
    if disabled {
        return theme.colors.text_muted;
    }

    // Semantic-ish mapping for common actions.
    if matches!(icon_path, Some("icons/trash.svg"))
        || label.contains("Delete")
        || label.contains("Drop")
        || label.contains("Remove")
    {
        return theme.colors.danger;
    }
    if matches!(icon_path, Some("icons/warning.svg"))
        || label.contains("Force")
        || label.contains("Discard")
    {
        return theme.colors.warning;
    }
    if matches!(icon_path, Some("icons/arrow_up.svg")) || label.starts_with("Push") {
        return theme.colors.success;
    }
    if matches!(icon_path, Some("icons/arrow_down.svg")) || label.starts_with("Pull") {
        return theme.colors.warning;
    }
    if matches!(icon_path, Some("icons/plus.svg")) || label.starts_with("Stage") {
        return theme.colors.success;
    }
    if matches!(icon_path, Some("icons/minus.svg")) || label.starts_with("Unstage") {
        return theme.colors.warning;
    }

    theme.colors.accent
}

fn context_menu_icon_path(icon: &str, label: &str) -> Option<&'static str> {
    let trimmed = icon.trim();
    let by_icon = match trimmed {
        "icons/link.svg" | "link" => Some("icons/link.svg"),
        "icons/unlink.svg" | "unlink" => Some("icons/unlink.svg"),
        "icons/plus.svg" => Some("icons/plus.svg"),
        "icons/minus.svg" => Some("icons/minus.svg"),
        "icons/question.svg" => Some("icons/question.svg"),
        "icons/warning.svg" => Some("icons/warning.svg"),
        "A" | "B" | "C" => None,
        "icons/check.svg" => Some("icons/check.svg"),
        "icons/git_branch.svg" => Some("icons/git_branch.svg"),
        "icons/arrow_down.svg" => Some("icons/arrow_down.svg"),
        "icons/arrow_up.svg" => Some("icons/arrow_up.svg"),
        "icons/broom.svg" => Some("icons/broom.svg"),
        "icons/tag.svg" => Some("icons/tag.svg"),
        "icons/trash.svg" => Some("icons/trash.svg"),
        "icons/refresh.svg" => Some("icons/refresh.svg"),
        "icons/open_external.svg" => Some("icons/open_external.svg"),
        "icons/file.svg" => Some("icons/file.svg"),
        "icons/folder.svg" => Some("icons/folder.svg"),
        "icons/copy.svg" => Some("icons/copy.svg"),
        "icons/box.svg" => Some("icons/box.svg"),
        "icons/menu.svg" => Some("icons/menu.svg"),
        "icons/swap.svg" => Some("icons/swap.svg"),
        "icons/arrow_right.svg" => Some("icons/arrow_right.svg"),
        "icons/infinity.svg" => Some("icons/infinity.svg"),
        "icons/arrow_left.svg" => Some("icons/arrow_left.svg"),
        "icons/undo.svg" => Some("icons/undo.svg"),
        "icons/pencil.svg" => Some("icons/pencil.svg"),
        "icons/cloud.svg" => Some("icons/cloud.svg"),
        "icons/computer.svg" => Some("icons/computer.svg"),
        _ => None,
    };
    if by_icon.is_some() {
        return by_icon;
    }

    if label.starts_with("Pull") {
        return Some("icons/arrow_down.svg");
    }
    if label.starts_with("Push") {
        return Some("icons/arrow_up.svg");
    }
    if label.contains("Delete") || label.contains("Drop") || label.contains("Remove") {
        return Some("icons/trash.svg");
    }
    if label.contains("Tag") {
        return Some("icons/tag.svg");
    }
    if label.contains("Open") && label.contains("location") {
        return Some("icons/folder.svg");
    }
    if label.contains("Open") {
        return Some("icons/open_external.svg");
    }
    if label.starts_with("Stage") {
        return Some("icons/plus.svg");
    }
    if label.starts_with("Unstage") {
        return Some("icons/minus.svg");
    }
    if label.contains("Squash") {
        return Some("icons/arrow_right.svg");
    }
    if label.contains("Edit") {
        return Some("icons/pencil.svg");
    }
    if label.contains("Resolve manually") {
        return Some("icons/pencil.svg");
    }
    if label.contains("Reset") {
        return Some("icons/refresh.svg");
    }
    if label.contains("Revert") {
        return Some("icons/undo.svg");
    }
    if label.contains("Copy") {
        return Some("icons/copy.svg");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_menu_icon_path_accepts_direct_svg_paths() {
        let paths = [
            "icons/link.svg",
            "icons/unlink.svg",
            "icons/plus.svg",
            "icons/minus.svg",
            "icons/question.svg",
            "icons/warning.svg",
            "icons/check.svg",
            "icons/git_branch.svg",
            "icons/arrow_down.svg",
            "icons/arrow_up.svg",
            "icons/broom.svg",
            "icons/tag.svg",
            "icons/trash.svg",
            "icons/refresh.svg",
            "icons/open_external.svg",
            "icons/file.svg",
            "icons/folder.svg",
            "icons/copy.svg",
            "icons/box.svg",
            "icons/menu.svg",
            "icons/swap.svg",
            "icons/arrow_right.svg",
            "icons/infinity.svg",
            "icons/arrow_left.svg",
            "icons/undo.svg",
            "icons/pencil.svg",
            "icons/cloud.svg",
            "icons/computer.svg",
        ];

        for path in paths {
            assert_eq!(context_menu_icon_path(path, "test"), Some(path));
        }
    }

    #[test]
    fn context_menu_icon_path_maps_named_link_icons() {
        assert_eq!(
            context_menu_icon_path("link", "test"),
            Some("icons/link.svg")
        );
        assert_eq!(
            context_menu_icon_path("unlink", "test"),
            Some("icons/unlink.svg")
        );
    }

    #[test]
    fn context_menu_icon_path_uses_label_fallbacks() {
        assert_eq!(
            context_menu_icon_path("", "Pull (merge)"),
            Some("icons/arrow_down.svg")
        );
        assert_eq!(
            context_menu_icon_path("", "Remove remote"),
            Some("icons/trash.svg")
        );
        assert_eq!(
            context_menu_icon_path("", "Squash into current"),
            Some("icons/arrow_right.svg")
        );
    }

    #[test]
    fn context_menu_icon_color_preserves_destructive_and_warning_semantics() {
        let theme = AppTheme::gitcomet_dark();
        assert_eq!(
            context_menu_icon_color(theme, false, "Delete branch", Some("icons/trash.svg")),
            theme.colors.danger
        );
        assert_eq!(
            context_menu_icon_color(theme, false, "Force push", Some("icons/warning.svg")),
            theme.colors.warning
        );
    }

    #[test]
    fn context_menu_icon_path_covers_all_context_menu_svg_icons() {
        let paths = [
            "icons/plus.svg",
            "icons/check.svg",
            "icons/git_branch.svg",
            "icons/arrow_down.svg",
            "icons/arrow_up.svg",
            "icons/broom.svg",
            "icons/tag.svg",
            "icons/trash.svg",
            "icons/refresh.svg",
            "icons/open_external.svg",
            "icons/file.svg",
            "icons/folder.svg",
            "icons/copy.svg",
            "icons/box.svg",
            "icons/infinity.svg",
            "icons/swap.svg",
            "icons/arrow_right.svg",
            "icons/arrow_left.svg",
            "icons/pencil.svg",
            "icons/link.svg",
            "icons/unlink.svg",
            "icons/warning.svg",
            "icons/minus.svg",
            "icons/cloud.svg",
            "icons/computer.svg",
        ];
        for path in paths {
            assert_eq!(
                context_menu_icon_path(path, "test"),
                Some(path),
                "missing direct SVG support for context-menu icon path: {path}"
            );
        }
    }
}
