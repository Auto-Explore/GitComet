//! Settings page building blocks: cards, headings, and the label/value rows a
//! settings page is made of, plus the navigation item that selects a page.
//!
//! Rows are plain elements; callers attach activation (`on_activate`) where a
//! row does something.

use crate::icons::svg_icon;
use crate::interaction::{ControlInteractionExt, InteractionState, InteractionStyle};
use crate::theme::{AppTheme, with_alpha};
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{AnyElement, CursorStyle, Div, FontWeight, Rgba, SharedString, Stateful, div, px};

const TOGGLE_TRACK_WIDTH_PX: f32 = 28.0;
const TOGGLE_TRACK_COMFORTABLE_WIDTH_PX: f32 = 36.0;
const TOGGLE_TRACK_HEIGHT_PX: f32 = 16.0;
const TOGGLE_TRACK_COMFORTABLE_HEIGHT_PX: f32 = 20.0;
const TOGGLE_TRACK_INSET_PX: f32 = 2.0;
const TOGGLE_KNOB_SIZE_PX: f32 = 12.0;
const TOGGLE_KNOB_COMFORTABLE_SIZE_PX: f32 = 16.0;
/// Width of the navigation column that lists settings pages.
pub const SETTINGS_NAV_COLUMN_WIDTH_PX: f32 = 200.0;

fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let t = t.clamp(0.0, 1.0);
    Rgba::new(
        a.red + (b.red - a.red) * t,
        a.green + (b.green - a.green) * t,
        a.blue + (b.blue - a.blue) * t,
        a.alpha + (b.alpha - a.alpha) * t,
    )
}

/// One choice in a settings list: a check mark when selected, the label, and
/// an optional one-line detail. The caller attaches activation.
pub fn settings_option_row(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    detail: Option<SharedString>,
    selected: bool,
    theme: AppTheme,
    ui_scale: impl Into<UiScale>,
) -> Stateful<Div> {
    let ui_scale = ui_scale.into();
    let id: SharedString = id.into();
    let debug_id = id.clone();
    let text_color = if selected {
        theme.colors.foreground.primary
    } else {
        theme.colors.foreground.secondary
    };
    let selected_bg = with_alpha(
        theme.colors.accent.foreground,
        if theme.is_dark { 0.16 } else { 0.10 },
    );

    div()
        .id(id)
        .debug_selector(move || debug_id.to_string())
        .w_full()
        .px_2()
        .py_1()
        .flex()
        .items_start()
        .gap_2()
        .rounded(px(theme.radii.row))
        .cursor(CursorStyle::PointingHand)
        .control_interaction(
            InteractionStyle::new(theme),
            InteractionState::default().selected(selected, selected_bg),
        )
        .child(
            div()
                .w(ui_scale.px(16.0))
                // Match the label's line box so the check mark centers on
                // the first text line instead of hugging the row's top.
                .h(ui_scale.px(20.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .when(selected, |d| {
                    d.child(svg_icon(
                        "icons/check.svg",
                        theme.colors.accent.foreground,
                        ui_scale.px(12.0),
                    ))
                }),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .gap_0p5()
                .child(
                    div()
                        .text_size(theme.ui_text(14.0))
                        .line_height(theme.ui_text(20.0))
                        .text_color(text_color)
                        .child(label.into()),
                )
                .when_some(detail, |this, detail| {
                    this.child(
                        div()
                            .text_size(theme.ui_text(12.0))
                            .text_color(theme.colors.foreground.secondary)
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(detail),
                    )
                }),
        )
}

pub fn settings_row_separator_color(theme: AppTheme) -> Rgba {
    mix(
        theme.colors.surface.canvas,
        theme.colors.stroke.subtle,
        if theme.is_dark { 0.14 } else { 0.10 },
    )
}

pub fn settings_dropdown_background(theme: AppTheme) -> Rgba {
    if theme.is_dark {
        mix(
            theme.colors.surface.raised,
            theme.colors.surface.canvas,
            0.58,
        )
    } else {
        mix(
            theme.colors.surface.raised,
            theme.colors.stroke.default,
            0.55,
        )
    }
}

pub fn settings_dropdown_border_color(theme: AppTheme) -> Rgba {
    if theme.is_dark {
        with_alpha(theme.colors.stroke.default, 0.98)
    } else {
        theme.colors.stroke.default
    }
}

fn selector(id: &SharedString, part: &str) -> impl Fn() -> String + 'static {
    let selector = format!("{id}_{part}");
    move || selector.clone()
}

/// The shell every label/value row shares: padding, separator, and overflow.
fn row_shell(id: &SharedString, theme: AppTheme) -> Stateful<Div> {
    let debug = id.to_string();
    div()
        .id(id.clone())
        .debug_selector(move || debug.clone())
        .w_full()
        .px_2()
        .pt_1()
        .pb_3()
        .flex()
        .items_center()
        .gap_2()
        .border_b_1()
        .border_color(settings_row_separator_color(theme))
        .overflow_hidden()
}

fn row_label(id: &SharedString, label: SharedString, theme: AppTheme) -> Div {
    div()
        .debug_selector(selector(id, "label"))
        .flex_1()
        .min_w(px(0.0))
        .overflow_hidden()
        .child(
            div()
                .text_size(theme.ui_text(14.0))
                .line_clamp(1)
                .whitespace_nowrap()
                .overflow_hidden()
                .child(label),
        )
}

fn card_shell(id: SharedString) -> Stateful<Div> {
    let debug = id.to_string();
    div()
        .id(id)
        .debug_selector(move || debug.clone())
        .w_full()
        .min_w(px(0.0))
        .flex()
        .flex_col()
        .gap_2()
}

fn card_title(title: SharedString, theme: AppTheme) -> Div {
    div()
        .px_2()
        .text_size(theme.ui_text(18.0))
        .font_weight(FontWeight::BOLD)
        .text_color(theme.colors.foreground.primary)
        .child(title)
}

/// A titled group of rows.
pub fn settings_card(
    id: impl Into<SharedString>,
    title: impl Into<SharedString>,
    theme: AppTheme,
) -> Stateful<Div> {
    card_shell(id.into()).child(card_title(title.into(), theme).pb_2())
}

/// A card whose title row ends in one compact action.
pub fn settings_card_with_action(
    id: impl Into<SharedString>,
    title: impl Into<SharedString>,
    action: impl IntoElement,
    theme: AppTheme,
) -> Stateful<Div> {
    card_shell(id.into()).child(
        div()
            .w_full()
            .min_w(px(0.0))
            .pb_2()
            .flex()
            .items_center()
            .gap_2()
            .child(
                card_title(title.into(), theme)
                    .flex_1()
                    .min_w(px(0.0))
                    .line_clamp(1)
                    .whitespace_nowrap()
                    .overflow_hidden(),
            )
            .child(div().flex_none().pr_2().child(action)),
    )
}

pub fn settings_subsection_heading(
    id: impl Into<SharedString>,
    title: impl Into<SharedString>,
    theme: AppTheme,
    ui_scale: UiScale,
) -> Stateful<Div> {
    let id = id.into();
    let debug = id.to_string();
    div()
        .id(id)
        .debug_selector(move || debug.clone())
        .w_full()
        .px_2()
        .pt(ui_scale.px(24.0))
        .pb_2()
        .text_size(theme.ui_text(14.0))
        .font_weight(FontWeight::BOLD)
        .text_color(theme.colors.foreground.primary)
        .child(title.into())
}

/// A bordered container for option lists and details below a row.
pub fn settings_detail_container(id: impl Into<SharedString>, theme: AppTheme) -> Stateful<Div> {
    let id = id.into();
    let debug = id.to_string();
    div()
        .id(id)
        .debug_selector(move || debug.clone())
        .w_full()
        .min_w(px(0.0))
        .flex()
        .flex_col()
        .rounded(px(theme.radii.row))
        .border_1()
        .border_color(settings_dropdown_border_color(theme))
        .bg(settings_dropdown_background(theme))
        .overflow_hidden()
}

/// A row that opens a list of choices; `value` is the current choice.
pub fn settings_summary_row(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    expanded: bool,
    theme: AppTheme,
    ui_scale: UiScale,
) -> Stateful<Div> {
    settings_summary_row_with_value_prefix(id, label, None, value, expanded, theme, ui_scale)
}

/// A summary row whose value leads with a small visual, such as a theme orb.
pub fn settings_summary_row_with_value_prefix(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    prefix: Option<AnyElement>,
    value: impl Into<SharedString>,
    expanded: bool,
    theme: AppTheme,
    ui_scale: UiScale,
) -> Stateful<Div> {
    let id = id.into();
    row_shell(&id, theme)
        .rounded(px(theme.radii.row))
        .cursor(CursorStyle::PointingHand)
        .control_interaction(InteractionStyle::new(theme), InteractionState::default())
        .child(row_label(&id, label.into(), theme))
        .child(
            div()
                .debug_selector(selector(&id, "value"))
                .min_w(px(0.0))
                .flex()
                .items_center()
                .justify_end()
                .gap_2()
                .text_size(theme.ui_text(14.0))
                .text_color(theme.colors.foreground.secondary)
                .overflow_hidden()
                .children(prefix.map(|prefix| div().flex_shrink_0().child(prefix)))
                .child(
                    div()
                        .min_w(px(0.0))
                        .line_clamp(1)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .child(value.into()),
                )
                .child(div().flex_shrink_0().child(svg_icon(
                    if expanded {
                        "icons/chevron_down.svg"
                    } else {
                        "icons/arrow_right.svg"
                    },
                    theme.colors.foreground.secondary,
                    ui_scale.px(12.0),
                ))),
        )
}

/// A row with an on/off switch; the whole row is the click target.
pub fn settings_toggle_row(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    enabled: bool,
    theme: AppTheme,
    ui_scale: UiScale,
) -> Stateful<Div> {
    let id = id.into();
    row_shell(&id, theme)
        .rounded(px(theme.radii.row))
        .cursor(CursorStyle::PointingHand)
        .control_interaction(InteractionStyle::new(theme), InteractionState::default())
        .child(row_label(&id, label.into(), theme))
        .child(
            div()
                .debug_selector(selector(&id, "value"))
                .flex_none()
                .flex()
                .items_center()
                .child(
                    // Toggle-switch visual; the whole row stays the click
                    // target, so this carries no handlers of its own.
                    div()
                        .w(ui_scale
                            .row_height(TOGGLE_TRACK_WIDTH_PX, TOGGLE_TRACK_COMFORTABLE_WIDTH_PX))
                        .h(ui_scale
                            .row_height(TOGGLE_TRACK_HEIGHT_PX, TOGGLE_TRACK_COMFORTABLE_HEIGHT_PX))
                        .rounded(px(theme.radii.pill))
                        .flex()
                        .items_center()
                        .p(ui_scale.px(TOGGLE_TRACK_INSET_PX))
                        .when(enabled, |track| {
                            track.justify_end().bg(theme.colors.accent.foreground)
                        })
                        .when(!enabled, |track| {
                            track.justify_start().bg(with_alpha(
                                theme.colors.foreground.secondary,
                                if theme.is_dark { 0.35 } else { 0.30 },
                            ))
                        })
                        .child(
                            div()
                                .size(ui_scale.row_height(
                                    TOGGLE_KNOB_SIZE_PX,
                                    TOGGLE_KNOB_COMFORTABLE_SIZE_PX,
                                ))
                                .rounded(px(theme.radii.pill))
                                .bg(gpui::rgba(0xFFFFFFF2)),
                        ),
                ),
        )
}

/// A read-only row: a label and a monospace value, text or an element such
/// as a selectable read-only input.
pub fn settings_info_row(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    value: impl IntoElement,
    theme: AppTheme,
) -> Stateful<Div> {
    let id = id.into();
    row_shell(&id, theme)
        .child(row_label(&id, label.into(), theme))
        .child(
            div()
                .debug_selector(selector(&id, "value"))
                .min_w(px(0.0))
                .flex()
                .items_center()
                .justify_end()
                .overflow_hidden()
                .child(
                    div()
                        .min_w(px(0.0))
                        .max_w_full()
                        .text_size(theme.ui_text(14.0))
                        .font_family(crate::bundled_fonts::UI_MONOSPACE_FONT_FAMILY)
                        .text_color(theme.colors.foreground.secondary)
                        .line_clamp(1)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .child(value),
                ),
        )
}

/// A row that opens an external page; `value` names where it goes.
pub fn settings_link_row(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    theme: AppTheme,
    ui_scale: UiScale,
) -> Stateful<Div> {
    let id = id.into();
    let debug = id.to_string();
    div()
        .id(id.clone())
        .debug_selector(move || debug.clone())
        .w_full()
        .px_2()
        .pt_1()
        .pb_3()
        .flex()
        .flex_col()
        .items_stretch()
        .gap_0p5()
        .rounded(px(theme.radii.row))
        .border_b_1()
        .border_color(settings_row_separator_color(theme))
        .cursor(CursorStyle::PointingHand)
        .control_interaction(InteractionStyle::new(theme), InteractionState::default())
        .child(
            div()
                .debug_selector(selector(&id, "label"))
                .min_w(px(0.0))
                .text_size(theme.ui_text(14.0))
                .child(label.into()),
        )
        .child(
            div()
                .debug_selector(selector(&id, "value"))
                .w_full()
                .min_w(px(0.0))
                .flex()
                .items_start()
                .gap_2()
                .text_size(theme.ui_text(14.0))
                .text_color(theme.colors.accent.foreground)
                .child(div().flex_1().min_w(px(0.0)).child(value.into()))
                .child(div().flex_shrink_0().child(svg_icon(
                    "icons/open_external.svg",
                    theme.colors.accent.foreground,
                    ui_scale.px(13.0),
                ))),
        )
}

/// A settings page in the navigation column. The caller attaches activation.
pub fn settings_nav_item(
    id: impl Into<SharedString>,
    icon: impl Into<SharedString>,
    label: impl Into<SharedString>,
    selected: bool,
    theme: AppTheme,
    ui_scale: UiScale,
) -> Stateful<Div> {
    let id = id.into();
    let debug = id.to_string();
    let icon_color = if selected {
        theme.colors.accent.foreground
    } else {
        theme.colors.foreground.secondary
    };
    div()
        .id(id)
        .debug_selector(move || debug.clone())
        .w_full()
        .px_2()
        .py_1()
        .flex()
        .items_center()
        .gap_2()
        .rounded(px(theme.radii.row))
        .cursor(CursorStyle::PointingHand)
        .overflow_hidden()
        .control_interaction(
            InteractionStyle::new(theme),
            InteractionState::default()
                .selected(selected, theme.colors.interaction.pressed_background),
        )
        .child(
            div()
                .flex_shrink_0()
                .child(svg_icon(icon.into(), icon_color, ui_scale.px(15.0))),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .text_size(theme.ui_text(14.0))
                .when(selected, |d| d.font_weight(FontWeight::MEDIUM))
                .text_color(theme.colors.foreground.primary)
                .line_clamp(1)
                .whitespace_nowrap()
                .overflow_hidden()
                .child(label.into()),
        )
}

#[cfg(test)]
mod option_row_tests {
    use super::*;

    struct Options {
        theme: AppTheme,
    }

    impl gpui::Render for Options {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            let scale = UiScale::from_percent(100);
            div()
                .w(px(320.0))
                .child(settings_option_row(
                    "option_on",
                    "On",
                    Some("Detail".into()),
                    true,
                    self.theme,
                    scale,
                ))
                .child(settings_option_row(
                    "option_off",
                    "Off",
                    None,
                    false,
                    self.theme,
                    scale,
                ))
        }
    }

    /// Only the selected row is filled; both span their list.
    #[gpui::test]
    fn the_selected_option_row_alone_is_filled(cx: &mut gpui::TestAppContext) {
        let _guard = crate::test_support::lock_visual_test();
        let theme = AppTheme::gitcomet_dark();
        let (_view, cx) = cx.add_window_view(|_, _| Options { theme });
        crate::test_support::redraw(cx);
        let selected = with_alpha(theme.colors.accent.foreground, 0.16);
        let fill_of = |cx: &mut gpui::VisualTestContext, selector| {
            crate::test_support::painted_control_quads(cx, selector)
                .into_iter()
                .map(|(fill, _)| fill)
                .collect::<Vec<_>>()
        };
        assert!(fill_of(cx, "option_on").contains(&selected.into()));
        assert!(!fill_of(cx, "option_off").contains(&selected.into()));
        assert_eq!(
            cx.debug_bounds("option_on").unwrap().size.width,
            cx.debug_bounds("option_off").unwrap().size.width
        );
    }
}
