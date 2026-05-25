use crate::theme::AppTheme;
use crate::ui_scale::UiScale;
use crate::view::tooltip_host::TooltipHost;
use gpui::prelude::*;
use gpui::{CursorStyle, Div, ElementId, Rgba, SharedString, Stateful, WeakEntity, div, px};

use super::control_height_md;
use super::{TextTruncationProfile, TruncatedText, TruncatedTextTooltipMode};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextMenuText {
    text: SharedString,
    profile: TextTruncationProfile,
    max_lines: Option<usize>,
    tooltip_mode: TruncatedTextTooltipMode,
}

impl ContextMenuText {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            profile: TextTruncationProfile::End,
            max_lines: None,
            tooltip_mode: TruncatedTextTooltipMode::None,
        }
    }

    pub fn path_single_line(text: impl Into<SharedString>) -> Self {
        Self::new(text)
            .profile(TextTruncationProfile::Path)
            .max_lines(1)
            .tooltip_mode(TruncatedTextTooltipMode::FullTextIfTruncated)
    }

    pub fn profile(mut self, profile: TextTruncationProfile) -> Self {
        self.profile = profile;
        self
    }

    pub fn max_lines(mut self, max_lines: usize) -> Self {
        self.max_lines = Some(max_lines.max(1));
        self
    }

    pub fn tooltip_mode(mut self, tooltip_mode: TruncatedTextTooltipMode) -> Self {
        self.tooltip_mode = tooltip_mode;
        self
    }

    fn resolved_max_lines(&self, default: usize) -> usize {
        self.max_lines.unwrap_or(default).max(1)
    }
}

impl AsRef<str> for ContextMenuText {
    fn as_ref(&self) -> &str {
        self.text.as_ref()
    }
}

impl From<SharedString> for ContextMenuText {
    fn from(value: SharedString) -> Self {
        Self::new(value)
    }
}

impl From<String> for ContextMenuText {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for ContextMenuText {
    fn from(value: &str) -> Self {
        Self::new(value.to_owned())
    }
}

pub fn context_menu(theme: AppTheme, content: impl IntoElement) -> Div {
    div()
        .w_full()
        .min_w_full()
        .flex()
        .flex_col()
        .items_stretch()
        .text_color(theme.colors.text)
        .child(content)
}

pub fn context_menu_header<V: 'static>(
    theme: AppTheme,
    ui_scale: impl Into<UiScale>,
    title: impl Into<ContextMenuText>,
    tooltip_host: Option<WeakEntity<TooltipHost>>,
    cx: &gpui::Context<V>,
) -> Div {
    let ui_scale = ui_scale.into();
    let scaled_px = |value| ui_scale.px(value);
    let title = title.into();
    let max_lines = title.resolved_max_lines(1);
    div()
        .w_full()
        .self_stretch()
        .px(scaled_px(8.0))
        .py(scaled_px(4.0))
        .text_xs()
        .line_height(scaled_px(14.0))
        .text_color(theme.colors.text_muted)
        .when(max_lines == 1, |s| s.whitespace_nowrap().overflow_hidden())
        .when(max_lines > 1, |s| s.line_clamp(max_lines))
        .child(context_menu_text_content(
            title,
            tooltip_host,
            cx,
            max_lines,
            theme.colors.text_muted,
        ))
}

pub fn context_menu_label<V: 'static>(
    theme: AppTheme,
    ui_scale: impl Into<UiScale>,
    text: impl Into<ContextMenuText>,
    tooltip_host: Option<WeakEntity<TooltipHost>>,
    cx: &gpui::Context<V>,
) -> Div {
    let ui_scale = ui_scale.into();
    let scaled_px = |value| ui_scale.px(value);
    let text = text.into();
    let max_lines = text.resolved_max_lines(2);
    div()
        .w_full()
        .self_stretch()
        .px(scaled_px(8.0))
        .pb(scaled_px(4.0))
        .text_sm()
        .line_height(scaled_px(18.0))
        .text_color(theme.colors.text)
        .when(max_lines == 1, |s| s.whitespace_nowrap().overflow_hidden())
        .when(max_lines > 1, |s| s.line_clamp(max_lines))
        .child(context_menu_text_content(
            text,
            tooltip_host,
            cx,
            max_lines,
            theme.colors.text,
        ))
}

pub fn context_menu_separator(theme: AppTheme, ui_scale: impl Into<UiScale>) -> Div {
    let ui_scale = ui_scale.into();
    let scaled_px = |value| ui_scale.px(value);
    div()
        .w_full()
        .self_stretch()
        .my(scaled_px(2.0))
        .border_t_1()
        .border_color(theme.colors.border)
}

pub fn context_menu_entry(
    id: impl Into<ElementId>,
    theme: AppTheme,
    ui_scale: impl Into<UiScale>,
    selected: bool,
    disabled: bool,
    icon: Option<SharedString>,
    label: impl Into<SharedString>,
    shortcut: Option<SharedString>,
) -> Stateful<Div> {
    let ui_scale = ui_scale.into();
    let scaled_px = |value| ui_scale.px(value);
    let label: SharedString = label.into();
    let icon_path = icon
        .as_ref()
        .and_then(|icon| context_menu_icon_path(icon.as_ref(), label.as_ref()));
    let icon_color = context_menu_icon_color(theme, disabled, label.as_ref(), icon_path);
    let text_color = if disabled {
        theme.colors.text_muted
    } else {
        theme.colors.text
    };

    let mut row = div()
        .id(id)
        .h(control_height_md(ui_scale))
        .w_full()
        .min_w_full()
        .self_stretch()
        .px(scaled_px(8.0))
        .flex()
        .items_center()
        .justify_between()
        .gap(scaled_px(8.0))
        .rounded(px(theme.radii.row))
        .text_color(text_color)
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
                .gap(scaled_px(8.0))
                .flex_1()
                .min_w(px(0.0))
                .child(
                    div()
                        .w(scaled_px(16.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .when_some(icon_path, |this, path| {
                            this.child(crate::view::icons::svg_icon(
                                path,
                                icon_color,
                                scaled_px(13.0),
                            ))
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_sm()
                        .line_height(scaled_px(18.0))
                        .text_color(text_color)
                        .line_clamp(1)
                        .child(label),
                ),
        );

    let mut end = div()
        .flex()
        .items_center()
        .gap(scaled_px(8.0))
        .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
        .text_xs()
        .line_height(scaled_px(14.0))
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
        "icons/stash.svg" => Some("icons/stash.svg"),
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

fn context_menu_text_content<V: 'static>(
    text: ContextMenuText,
    tooltip_host: Option<WeakEntity<TooltipHost>>,
    cx: &gpui::Context<V>,
    max_lines: usize,
    text_color: Rgba,
) -> impl IntoElement {
    if max_lines == 1 {
        let mut truncated = TruncatedText::new(text.text.clone())
            .profile(text.profile)
            .text_color(text_color);
        if let (Some(tooltip_host), TruncatedTextTooltipMode::FullTextIfTruncated) =
            (tooltip_host, text.tooltip_mode)
        {
            truncated = truncated.full_text_tooltip(tooltip_host);
        }
        return truncated.render(cx).into_any_element();
    }

    div()
        .text_color(text_color)
        .child(text.text)
        .into_any_element()
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
            "icons/stash.svg",
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
            "icons/stash.svg",
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
