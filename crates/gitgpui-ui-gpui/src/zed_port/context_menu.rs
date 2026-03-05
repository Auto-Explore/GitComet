//! Context menu building blocks (Zed-inspired styling).

use crate::theme::AppTheme;
use gpui::prelude::*;
use gpui::{AnyElement, CursorStyle, Div, ElementId, FontWeight, SharedString, Stateful, div, px};

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

#[allow(clippy::too_many_arguments)]
pub fn context_menu_entry(
    id: impl Into<ElementId>,
    theme: AppTheme,
    selected: bool,
    disabled: bool,
    icon: Option<SharedString>,
    label: impl Into<SharedString>,
    shortcut: Option<SharedString>,
    has_submenu: bool,
) -> Stateful<Div> {
    let label: SharedString = label.into();
    let icon_color = context_menu_icon_color(
        theme,
        disabled,
        label.as_ref(),
        icon.as_ref().map(|icon| icon.as_ref()),
    );

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
                        .when_some(icon, |this, icon| {
                            this.child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(icon_color)
                                    .child(icon),
                            )
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
    if has_submenu {
        end = end.child("›");
    }
    row = row.child(end);

    if disabled {
        row = row
            .text_color(theme.colors.text_muted)
            .cursor(CursorStyle::Arrow);
    }

    row
}

#[allow(clippy::too_many_arguments)]
pub fn context_menu_entry_with_end_slot(
    id: impl Into<ElementId>,
    theme: AppTheme,
    selected: bool,
    disabled: bool,
    icon: Option<SharedString>,
    label: impl Into<SharedString>,
    end_slot: Option<AnyElement>,
    shortcut: Option<SharedString>,
    has_submenu: bool,
) -> Stateful<Div> {
    let label: SharedString = label.into();
    let icon_color = context_menu_icon_color(
        theme,
        disabled,
        label.as_ref(),
        icon.as_ref().map(|icon| icon.as_ref()),
    );

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
                        .when_some(icon, |this, icon| {
                            this.child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(icon_color)
                                    .child(icon),
                            )
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
        .text_xs()
        .text_color(theme.colors.text_muted);

    if let Some(end_slot) = end_slot {
        end = end.child(end_slot);
    }

    if let Some(shortcut) = shortcut {
        end = end.child(div().font_family("monospace").child(shortcut));
    }
    if has_submenu {
        end = end.child(div().font_family("monospace").child("›"));
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
    icon: Option<&str>,
) -> gpui::Rgba {
    if disabled {
        return theme.colors.text_muted;
    }

    if let Some(icon) = icon {
        // Semantic-ish mapping for common actions.
        if icon == "🗑"
            || label.contains("Delete")
            || label.contains("Drop")
            || label.contains("Remove")
        {
            return theme.colors.danger;
        }
        if icon == "⚠" || label.contains("Force") || label.contains("Discard") {
            return theme.colors.warning;
        }
        if icon == "↑" || icon == "⇡" || label.starts_with("Push") {
            return theme.colors.success;
        }
        if icon == "↓" || label.starts_with("Pull") {
            return theme.colors.warning;
        }
        if icon == "+" || label.starts_with("Stage") {
            return theme.colors.success;
        }
        if icon == "−" || label.starts_with("Unstage") {
            return theme.colors.warning;
        }
    }

    theme.colors.accent
}
